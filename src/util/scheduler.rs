use std::sync::Arc;

use async_trait::async_trait;
use redis::{
  Commands, Connection, FromRedisValue, 
  RedisError, RedisResult, Value,
  transaction
};
use serde::{Deserialize, Serialize};
use serenity::{
  http::Http,
  prelude::{TypeMapKey},
  model::{
    channel::ReactionType,
    id::{ChannelId, UserId}
  }
};
use tokio::sync::Mutex;
use uuid::Uuid;

use std::{
  io::{Error, ErrorKind::Other}
};

macro_rules! redis_error {
  ($message:expr) => {
      Err(RedisError::from(Error::new(Other, $message)))
  };
}


// adapted from https://github.com/stayingqold/Poll-Bot/blob/master/cogs/poll.py 
pub const EMOJI_ORDER: &[&str] = &[
  "1️⃣", "2️⃣", "3️⃣", "4️⃣", "5️⃣", "6️⃣", "7️⃣", "8️⃣", "9️⃣", "🔟",
  "🇦", "🇧", "🇨", "🇩", "🇪", "🇫", "🇬", "🇭", "🇮", "🇯"
];

fn vote_str(count: usize) -> &'static str {
  if count == 1 {
    "vote"
  } else {
    "votes"
  }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PollCache {
  pub author: u64,
  pub id: String
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Poll {
  pub author: u64,
  pub channel: u64,
  pub message: u64,
  pub topic: String
}

#[async_trait]
impl Callable<Arc<Http>> for Poll {
  async fn call(&self, http: &Arc<Http>) {
    let channel_id = ChannelId(self.channel);

    let message = match channel_id.message(http, self.message).await {
      Ok(msg) => msg,
      Err(error) => {
        if let Ok(user) = UserId(self.author).to_user(http).await {
          let _ = user.dm(http, |m| {
            m.content(&format!("Failed to conclude poll {}: {}", self.topic, error))
          });
        }

        return;
      }
    };

    if message.embeds.is_empty() {
      return;
    }

    let mut options: Vec<&str> = vec![];

    if let Some(content) = &message.embeds[0].description {
      for option in content.split('\n') {
        let index = match option.find('.') {
          Some(loc) => loc + 2,
          None => 0
        };

        options.push(&option[index..]);
      }
    }

    let mut results: Vec<(usize, &str)> = vec![];

    for reaction in message.reactions.iter() {
      if let ReactionType::Unicode(emoji) = reaction.reaction_type.clone() {
        let possible_idx = EMOJI_ORDER.iter()
          .position(|e| *e == emoji);

        if let Some(idx) = possible_idx {
          if idx < options.len() {
            results.push(((reaction.count - 1) as usize, options[idx]));
          }
        }
      }
    }

    results.sort_by(|a, b| b.cmp(a));

    let mut wins: Vec<&str> = vec![results[0].1];
    let max_count = results[0].0;
    let max_vote_msg = vote_str(max_count);

    if results.len() > 1 {
      for item in &results[1..] {
        if item.0 == max_count {
          wins.push(item.1);
        }
      }
    }

    let mut result_msg = format!("results of {}\n", self.topic);

    if wins.len() > 1 {
      let joined_str = wins.join(", ");
      result_msg += &format!("**Tie between {}** ({} {} each)",
        joined_str, &max_count, max_vote_msg);
    } else {
      result_msg += &format!("**{}** wins! ({} {})",
        wins[0], &max_count, max_vote_msg);
    }

    if results.len() > wins.len() {
      result_msg += "\n\n>>> ";
      for item in &results[wins.len()..] {
        let vote_msg = vote_str(item.0);
        result_msg += &format!("**{}** ({} {})\n", 
          item.1, item.0, vote_msg);
      }
    }


    let _ = channel_id.say(http, result_msg).await;
  }
}
  
pub struct Scheduler {
  connection: Connection,
  jobs_key: String,
  schedule_key: String,
  rng: Box<dyn Fn() -> String + Send>
}

#[derive(Debug)]
pub struct MyVec {
  v: Vec<Vec<u8>>,
}

impl FromRedisValue for MyVec {
  fn from_redis_value(v: &Value) -> RedisResult<MyVec> {
    match *v {
      Value::Data(ref bytes) => {
        Ok(MyVec{
          v: vec![bytes.to_owned()]
        })
      },
      Value::Bulk(ref items) => {
        match FromRedisValue::from_redis_values(items) {
          Ok(vec) => Ok(MyVec{v: vec}),
          Err(error) => redis_error!(error)
        }
      }
      Value::Nil => Ok(MyVec {v: vec![]}),
      _ => redis_error!("Response type not vector compatible."),
    }
  }
}

const JOBS_KEY: &str = "jobs";
const SCHEDULE_KEY: &str = "schedule";

fn generic_rng() -> String {
  Uuid::new_v4().to_simple().to_string()
}

impl Scheduler {
  pub fn new(connection: Connection, method: Option<Box<dyn Fn() -> String + Send>>,
    jobs_key: Option<&str>, schedule_key: Option<&str>) -> Scheduler {
    let rng = match method {
      Some(random) => random,
      None => Box::new(generic_rng)
    };

    Scheduler {
      connection,
      jobs_key: String::from(jobs_key.unwrap_or(JOBS_KEY)),
      schedule_key: String::from(schedule_key.unwrap_or(SCHEDULE_KEY)),
      rng,
    }
  }

  pub fn clear_jobs(&mut self) -> RedisResult<()> {
    redis::pipe()
      .atomic()
      .del(&[&self.jobs_key, &self.schedule_key])
      .query(&mut self.connection)
  }
  
  pub fn clear_ready_jobs(&mut self, timestamp: i64) -> RedisResult<()> {
    let jobs_key = &self.jobs_key;
    let schedule_key = &self.schedule_key;

    let _ = transaction(&mut self.connection, &[jobs_key, schedule_key], |con, pipe| {

      let ready_jobs: Vec<String> = con.zrangebyscore(
        schedule_key, "-inf", timestamp)?;
      
      if ready_jobs.is_empty() {
        return Ok(Some(()));
      }            

      pipe
        .hdel(jobs_key, &ready_jobs[..]).ignore()
        .zrembyscore(schedule_key, "-inf", timestamp).ignore()
        .query(con)
    })?;

    Ok(())
  }

  pub fn edit_job(&mut self, task: &Poll, id: &str, time: Option<i64>) -> RedisResult<()> {
    let task = match bincode::serialize(task) {
      Ok(serialized) => serialized,
      Err(error) => return redis_error!(error)    
    };

    let jobs_key = &self.jobs_key;
    let sched_key = &self.schedule_key;

    transaction(&mut self.connection, &[jobs_key, sched_key], |con, pipe| {
      let exists: u8 = con.hexists(jobs_key, id)?;

      if exists == 0 {
        return redis_error!(format!("No job found for {}", id));
      }

      let mut pipeline = pipe.hset(jobs_key, id, &task[..]).ignore();

      if let Some(new_time) = time {
        pipeline = pipeline.zadd(sched_key, id, new_time).ignore()
      }

      pipeline.query(con)
    })
  }

  pub fn get_job(&mut self, id: &str) -> RedisResult<Poll> {
    let task: Option<Vec<u8>> = self.connection.hget(&self.jobs_key, id)?;

    let task = match task {
      Some(evt) => evt,
      None => return redis_error!(format!("No job found for {}", id))
    };

    match bincode::deserialize(&task) {
      Ok(result) => Ok(result),
      Err(error) =>  redis_error!(error)
    }    
  }

  pub fn get_and_clear_ready_jobs(&mut self, timestamp: i64) -> RedisResult<Vec<Poll>> {
    let jobs_key = &self.jobs_key;
    let schedule_key = &self.schedule_key;

    let jobs_as_string: Vec<MyVec> = transaction(&mut self.connection,
      &[jobs_key, schedule_key], |con, pipe| {

      let ready_jobs: Vec<String> = con.zrangebyscore(schedule_key, "-inf", timestamp)?;
      
      if ready_jobs.is_empty() {
        return Ok(Some(vec![]));
      }            

      pipe
        .hget(jobs_key, &ready_jobs[..])
        .hdel(jobs_key, &ready_jobs[..]).ignore()
        .zrembyscore(schedule_key, "-inf", timestamp).ignore()
        .query(con)
    })?;

    let mut jobs_as_t: Vec<Poll> = Vec::new();

    for my_vec in jobs_as_string.iter() {
      for job in my_vec.v.iter() {
        match bincode::deserialize(job) {
          Ok(result) => jobs_as_t.push(result),
          Err(error) => return Err(RedisError::from(Error::new(Other, error)))
        }                
      }
    }

    Ok(jobs_as_t)
  }

  pub fn get_ready_jobs(&mut self, timestamp: i64) -> RedisResult<Vec<Poll>> {
    let jobs_key = &self.jobs_key;
    let schedule_key = &self.schedule_key;

    let jobs_as_string: Vec<MyVec> = redis::transaction(&mut self.connection,
      &[jobs_key, schedule_key], |con, pipe| {

      let ready_jobs: Vec<String> = con.zrangebyscore(
        schedule_key, "-inf", timestamp)?;
      
      if ready_jobs.is_empty() {
        return Ok(Some(vec![]));
      }            

      pipe.hget(jobs_key, ready_jobs).query(con)
    })?;

    let mut jobs_as_t: Vec<Poll> = Vec::new();

    for my_vec in jobs_as_string.iter() {
      for job in my_vec.v.iter() {
        match bincode::deserialize(job) {
          Ok(result) => jobs_as_t.push(result),
          Err(error) => return Err(RedisError::from(Error::new(Other, error)))
        }
      }
    }

    Ok(jobs_as_t)
  }

  pub fn remove_job(&mut self, job_id: &str, message_id: u64) -> RedisResult<()> {
    let jobs_key = &self.jobs_key;
    let sched_key = &self.schedule_key;

    let _: () = transaction(&mut self.connection, &[jobs_key, sched_key], |con, pipe| {
      let job_score: Option<i64> = con.zscore(sched_key, job_id)?;

      match job_score {
        None => Ok(Some(())),
        Some(_) => {
          pipe
            .del(message_id)
            .hdel(jobs_key, job_id)
            .zrem(sched_key, job_id)
            .query(con)
        },
      }
    })?;

    Ok(())
  }

  pub fn schedule_job(&mut self, task: &Poll, timestamp: i64, duration: i64) ->  RedisResult<String>  {
    let message_id = task.message;
    let author_id = task.author;

    let task = match bincode::serialize(task) {
      Ok(serialized) => serialized,
      Err(error) => return redis_error!(error)    
    };

    let rng_generator = &self.rng;

    let mut new_id = rng_generator();

    let cache = PollCache { author: author_id, id: new_id.clone() };
    let cache = match bincode::serialize(&cache) {
      Ok(serialized) => serialized,
      Err(error) => return redis_error!(error)
    };
    
    let jobs_key = &self.jobs_key;
    let schedule_key = &self.schedule_key;

    let _: () = transaction(&mut self.connection, &[jobs_key, schedule_key], |con, pipe| {
      loop {
        if !con.hexists(jobs_key, new_id.to_string())? {
          break pipe
            .zadd(schedule_key, &new_id, timestamp)
            .hset(jobs_key, &new_id, &task[..])
            .set_ex(message_id, cache.clone(), duration as usize)
            .query(con);
        }

        new_id = rng_generator();
      }
    })?;

    Ok(new_id)
  }
}

#[async_trait]
pub trait Callable<T> {
  async fn call(&self, arg: &T);
}

pub struct RedisSchedulerKey;
impl TypeMapKey for RedisSchedulerKey {
  type Value = Arc<Mutex<Scheduler>>;
}

pub struct RedisWrapper(pub Connection);

pub struct RedisConnectionKey;
impl TypeMapKey for RedisConnectionKey {
  type Value = Arc<Mutex<RedisWrapper>>;
}
