extern crate redis;

use redis::{Client, Commands, Connection, IntoConnectionInfo, RedisError, RedisResult};
use serde::{Serialize, de::DeserializeOwned};
use uuid::Uuid;

use std::io::{Error, ErrorKind::Other};
use std::marker::PhantomData;

pub struct Scheduler<T: Callable + DeserializeOwned + Serialize> {
    connection: Connection,
    jobs_key: String,
    schedule_key: String,
    
    resource_type: PhantomData<T>,
}

const JOBS_KEY: &str = "jobs";
const SCHEDULE_KEY: &str = "schedule";

impl<T: Callable + DeserializeOwned + Serialize> Scheduler<T> {
    pub fn create<R: IntoConnectionInfo>(args: R, 
        jobs_key: Option<&str>, schedule_key: Option<&str>) -> RedisResult<Scheduler<T>> {


        let client = Client::open(args)?;
        let connection = client.get_connection()?;

        Ok(Scheduler {
            connection,
            jobs_key: String::from(jobs_key.unwrap_or(JOBS_KEY)),
            schedule_key: String::from(schedule_key.unwrap_or(SCHEDULE_KEY)),

            resource_type: PhantomData
        })
    }

    pub fn get_job(&mut self, id: &str) -> redis::RedisResult<T> {
        let event: Vec<u8> = match self.connection.hget(&self.jobs_key, id) {
            Ok(result) => result,
            Err(error) => return Err(error)
        };
    
        match bincode::deserialize(&event) {
            Ok(result) => Ok(result),
            Err(error) =>  Err(RedisError::from(Error::new(Other, error)))
        }    
    }

    pub fn schedule_job(&mut self, task: &T, timestamp: i64) ->  RedisResult<String>  {
        let task = match bincode::serialize(task) {
            Ok(serialized) => serialized,
            Err(error) => {
                return Err(RedisError::from(Error::new(Other, error)))
            }
        };

        let mut new_id = Uuid::new_v4().to_simple().to_string();

        let jobs_key = &self.jobs_key;
        let schedule_key = &self.schedule_key;

        let _: () = redis::transaction(&mut self.connection, &[schedule_key], |con, pipe| {
            loop {
                
                if !con.hexists(jobs_key, new_id.to_string())? {
                    break pipe
                        .zadd(schedule_key, &new_id, timestamp)
                        .hset(jobs_key, &new_id, task.as_slice())
                        .query(con);
                }

                new_id = Uuid::new_v4().to_simple().to_string();
            }
        })?;

        Ok(new_id)
    }
}

pub trait Callable {
    fn call(&self);
}
