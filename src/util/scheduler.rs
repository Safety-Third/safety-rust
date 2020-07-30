use redis::{
  Commands, Connection, FromRedisValue, 
  RedisError, RedisResult, Value,
  transaction
};
use serde::{Serialize, de::DeserializeOwned};
use uuid::Uuid;

use std::io::{Error, ErrorKind::Other};
use std::marker::PhantomData;

macro_rules! redis_error {
  ($message:expr) => {
      Err(RedisError::from(Error::new(Other, $message)))
  };
}
  
pub struct Scheduler<T: Callable<A> + DeserializeOwned + Serialize, A> {
  connection: Connection,
  jobs_key: String,
  schedule_key: String,
  
  argument_type: PhantomData<A>,
  resource_type: PhantomData<T>,
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


impl<T: Callable<A> + DeserializeOwned + Serialize, A> Scheduler<T, A> {
  pub fn new(connection: Connection, 
      jobs_key: Option<&str>, schedule_key: Option<&str>) -> Scheduler<T, A> {

    Scheduler {
      connection,
      jobs_key: String::from(jobs_key.unwrap_or(JOBS_KEY)),
      schedule_key: String::from(schedule_key.unwrap_or(SCHEDULE_KEY)),

      argument_type: PhantomData,
      resource_type: PhantomData
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

  pub fn edit_job(&mut self, task: &T, id: &str, time: Option<i64>) -> RedisResult<()> {
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

  pub fn get_job(&mut self, id: &str) -> RedisResult<T> {
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

  pub fn get_and_clear_ready_jobs(&mut self, timestamp: i64) -> RedisResult<Vec<T>> {
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

    let mut jobs_as_t: Vec<T> = Vec::new();

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

  pub fn get_ready_jobs(&mut self, timestamp: i64) -> RedisResult<Vec<T>> {
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

    let mut jobs_as_t: Vec<T> = Vec::new();

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

  pub fn remove_job(&mut self, job_id: &str) -> RedisResult<()> {
    let jobs_key = &self.jobs_key;
    let sched_key = &self.schedule_key;

    // let job_score: 
    let _: () = transaction(&mut self.connection, &[jobs_key, sched_key], |con, pipe| {
      let job_score: Option<i64> = con.zscore(sched_key, job_id)?;

      match job_score {
        None => Ok(Some(())),
        Some(_) => {
          pipe
            .hdel(jobs_key, job_id)
            .zrem(sched_key, job_id)
            .query(con)
        },
      }
    })?;

    Ok(())
  }

  pub fn schedule_job(&mut self, task: &T, timestamp: i64) ->  RedisResult<String>  {
    let task = match bincode::serialize(task) {
      Ok(serialized) => serialized,
      Err(error) => return redis_error!(error)    
    };

    let mut new_id = Uuid::new_v4().to_simple().to_string();

    let jobs_key = &self.jobs_key;
    let schedule_key = &self.schedule_key;

    let _: () = transaction(&mut self.connection, &[jobs_key, schedule_key], |con, pipe| {
      loop {
        if !con.hexists(jobs_key, new_id.to_string())? {
          break pipe
            .zadd(schedule_key, &new_id, timestamp)
            .hset(jobs_key, &new_id, &task[..])
            .query(con);
        }

        new_id = Uuid::new_v4().to_simple().to_string();
      }
    })?;

    Ok(new_id)
  }
}
  
pub trait Callable<T> {
  fn call(&self, arg: &T);
}
  
  