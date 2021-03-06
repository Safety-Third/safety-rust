use std::sync::Arc;

use async_trait::async_trait;
use redis::Connection;
use serde::{Serialize, Deserialize};
use serenity::{
  http::Http,
  prelude::{TypeMapKey},
  model::{
    channel::ReactionType,
    id::{ChannelId, UserId},
    misc::Mentionable
  }
};
use tokio::sync::Mutex;

use crate::util::scheduler::{Callable, Scheduler as RedisScheduler};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Event {
  pub author: u64,
  pub channel: u64,
  pub event: String,
  pub members: Vec<u64>,
  pub time: String,
}

impl Event {
  #[inline]
  pub fn members_and_author(&self) -> String {
    let author = UserId(self.author).mention();
    if self.members.is_empty() {
      author.to_string()
    } else {
      format!("{}, {}", author, self.members())
    }
  }

  #[inline]
  pub fn members(&self) -> String {
    self.members
      .iter()
      .map(|member| UserId(*member).mention().to_string())
      .collect::<Vec<String>>()
      .join(", ")
  }
}

#[async_trait]
impl Callable<Arc<Http>> for Event {
  async fn call(&self, http: &Arc<Http>) {
    let members = self.members();
    let author = UserId(self.author).mention();

    let send_result = ChannelId(self.channel).send_message(http, |m| {
      m.content(format!("Time for **{}** by {}\n{}", self.event, &author, &members))
    }).await;

    if let Err(error) = send_result {
      if let Ok(user) = UserId(self.author).to_user(http).await {
        let _ = user.dm(http, |m| {
          m.content(&format!("Failed to hold event {}: {}", self.event, error))
        }).await;
      }
    }
  }
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

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum Task {
  Event(Event),
  Poll(Poll)
}

#[async_trait]
impl Callable<Arc<Http>> for Task {
  async fn call(&self, http: &Arc<Http>) {
    match self {
      Task::Event(item) => item.call(&http).await,
      Task::Poll(item) => item.call(&http).await
    };
  }
}


pub struct RedisSchedulerKey;
impl TypeMapKey for RedisSchedulerKey {
  type Value = Arc<Mutex<RedisScheduler<Task, Arc<Http>>>>;
}

pub struct RedisWrapper(pub Connection);

pub struct RedisConnectionKey;
impl TypeMapKey for RedisConnectionKey {
  type Value = Arc<Mutex<RedisWrapper>>;
}
