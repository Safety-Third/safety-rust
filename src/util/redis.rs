use futures_core::future::BoxFuture;
use redis::{aio::ConnectionLike, cmd, pipe, Pipeline, RedisResult, ToRedisArgs};

pub async fn transaction<
  'a,
  'fut: 'a,
  C: ConnectionLike,
  K: ToRedisArgs,
  T: Sized,
  F: FnMut(&'a mut C, &mut Pipeline) -> BoxFuture<'a, RedisResult<Option<T>>>,
>(
  con: &'fut mut C,
  keys: &[K],
  mut func: F,
) -> RedisResult<T> {
  loop {
    cmd("WATCH").arg(keys).query_async::<C, ()>(con).await?;
    let mut pipe = pipe();

    let response = {
      let awaited = func(con, pipe.atomic());
      awaited.await?
    };
    match response {
      None => {
        continue;
      }
      Some(response) => {
        cmd("UNWWATCH").query_async::<C, ()>(con).await?;

        return Ok(response);
      }
    }
  }
}
