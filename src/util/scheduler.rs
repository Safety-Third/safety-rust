use std::{io::{Error, ErrorKind::Other}, sync::Arc};

use async_trait::async_trait;
use redis::{
  aio::Connection,
  AsyncCommands, FromRedisValue,
  RedisError, RedisResult, Value,
  cmd, pipe
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

use super::rng::random_id;

macro_rules! redis_error {
  ($message:expr) => {
      Err(RedisError::from(Error::new(Other, $message)))
  };
}

// adapted from https://github.com/mitsuhiko/redis-rs/issues/353
macro_rules! async_transaction {
  ($conn:expr, $keys:expr, $body:expr) => {
      loop {
          cmd("WATCH").arg($keys).query_async($conn).await?;

          if let Some(response) = $body {
              cmd("UNWATCH").query_async($conn).await?;
              break response;
          }
      }
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
  schedule_key: String
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
  pub fn new(connection: Connection,
    jobs_key: Option<&str>, schedule_key: Option<&str>) -> Scheduler {

    Scheduler {
      connection,
      jobs_key: String::from(jobs_key.unwrap_or(JOBS_KEY)),
      schedule_key: String::from(schedule_key.unwrap_or(SCHEDULE_KEY))
    }
  }

  pub async fn clear_jobs(&mut self) -> RedisResult<()> {
    pipe()
      .atomic()
      .del(&[&self.jobs_key, &self.schedule_key])
      .query_async(&mut self.connection)
      .await
  }

  pub async fn clear_ready_jobs(&mut self, timestamp: i64) -> RedisResult<()> {
    let jobs_key = &self.jobs_key;
    let schedule_key = &self.schedule_key;

    let _: () = async_transaction!(&mut self.connection, &[jobs_key, schedule_key], {
      let ready_jobs: Vec<String> = self.connection.zrangebyscore(
        schedule_key, "-inf", timestamp).await?;

      if ready_jobs.is_empty() {
        return Ok(());
      }

      pipe().atomic()
        .hdel(jobs_key, &ready_jobs[..]).ignore()
        .zrembyscore(schedule_key, "-inf", timestamp).ignore()
        .query_async(&mut self.connection)
        .await?
    });


    Ok(())
  }

  pub async fn edit_job(&mut self, task: &Poll, id: &str, time: Option<i64>) -> RedisResult<()> {
    let task = match bincode::serialize(task) {
      Ok(serialized) => serialized,
      Err(error) => return redis_error!(error)
    };

    let jobs_key = &self.jobs_key;
    let sched_key = &self.schedule_key;

    let con = &mut self.connection;

    let _: () = async_transaction!(con, &[jobs_key, sched_key], {
      let exists: u8 = con.hexists(jobs_key, id).await?;

      if exists == 0 {
        return redis_error!(format!("No job found for {}", id));
      }

      if let Some(new_time) = time {
        pipe().atomic()
          .hset(jobs_key, id, &task[..]).ignore()
          .zadd(sched_key, id, new_time).ignore()
          .query_async(con).await?
      } else {
        pipe().atomic()
          .hset(jobs_key, id, &task[..]).ignore()
          .query_async(con).await?
      }
    });

    Ok(())
  }

  pub async fn get_job(&mut self, id: &str) -> RedisResult<Poll> {
    let task: Option<Vec<u8>> = self.connection.hget(&self.jobs_key, id).await?;

    let task = match task {
      Some(evt) => evt,
      None => return redis_error!(format!("No job found for {}", id))
    };

    match bincode::deserialize(&task) {
      Ok(result) => Ok(result),
      Err(error) =>  redis_error!(error)
    }
  }

  pub async fn get_and_clear_ready_jobs(&mut self, timestamp: i64) -> RedisResult<Vec<Poll>> {
    let jobs_key = &self.jobs_key;
    let schedule_key = &self.schedule_key;
    let con = &mut self.connection;

    let jobs_as_string: Vec<MyVec> = async_transaction!(con, &[jobs_key, schedule_key], {
      let ready_jobs: Vec<String> = con.zrangebyscore(schedule_key, "-inf", timestamp).await?;

      if ready_jobs.is_empty() {
        return Ok(vec![]);
      }

      pipe().atomic()
        .hget(jobs_key, &ready_jobs[..])
        .hdel(jobs_key, &ready_jobs[..]).ignore()
        .zrembyscore(schedule_key, "-inf", timestamp).ignore()
        .query_async(con)
        .await?
    });

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

  pub async fn get_ready_jobs(&mut self, timestamp: i64) -> RedisResult<Vec<Poll>> {
    let jobs_key = &self.jobs_key;
    let schedule_key = &self.schedule_key;

    let con = &mut self.connection;

    let jobs_as_string: Vec<MyVec> = async_transaction!(con, &[jobs_key, schedule_key], {

      let ready_jobs: Vec<String> = con.zrangebyscore(
        schedule_key, "-inf", timestamp).await?;

      if ready_jobs.is_empty() {
        return Ok(vec![]);
      }

      pipe().atomic()
        .hget(jobs_key, ready_jobs)
        .query_async(con)
        .await?
    });

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

  pub async fn remove_job(&mut self, message_id: u64) -> RedisResult<()> {
    let jobs_key = &self.jobs_key;
    let sched_key = &self.schedule_key;

    let job_id: Option<String> = self.connection.get(message_id).await?;

    if let Some(job) = job_id {
      let con = &mut self.connection;
      let _: () = async_transaction!(con, &[jobs_key, sched_key], {
        let job_score: Option<i64> = con.zscore(sched_key, &job).await?;

        match job_score {
          None => pipe().atomic().query_async(con).await?,
          Some(_) => {
            pipe().atomic()
              .del(message_id)
              .hdel(jobs_key, &job)
              .zrem(sched_key, &job)
              .query_async(con)
              .await?
          }
        }
      });
    }

    Ok(())
  }

  pub async fn schedule_job(&mut self, task: &Poll, timestamp: i64, duration: i64) ->  RedisResult<String>  {
    let message_id = task.message;

    let task = match bincode::serialize(task) {
      Ok(serialized) => serialized,
      Err(error) => return redis_error!(error)
    };

    let mut new_id = random_id();

    let jobs_key = &self.jobs_key;
    let schedule_key = &self.schedule_key;
    let con = &mut self.connection;

    let _: () = async_transaction!(con, &[jobs_key, schedule_key, &message_id.to_string()], {
      loop {
        if !con.hexists(jobs_key, new_id.to_string()).await? {
          break pipe().atomic()
            .zadd(schedule_key, &new_id, timestamp)
            .hset(jobs_key, &new_id, &task[..])
            .set_ex(message_id, &new_id, duration as usize)
            .query_async(con)
            .await?;
        }

        new_id = random_id();
      }
    });

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
