use crate::{MyVec, RedisConnectionKey};
use std::{sync::Arc, time::Duration};

use chrono::Utc;
use chrono_tz::EST5EDT;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use serenity::{
  framework::standard::{macros::command, Args, CommandResult},
  http::Http,
  model::prelude::*,
  prelude::*,
};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Briefing {
  pub author: String,
  pub heading: String,
  pub body: Option<String>,
  pub time: String,
}

pub struct BriefingGuildKey;
impl TypeMapKey for BriefingGuildKey {
  type Value = u64;
}

pub const BRIEFING_REPLIES: &[&str] = &["✅", "❌"];

pub const BRIEFING_KEY: &str = "safety:briefings";

#[command]
#[only_in("dms")]
#[example("This is your header! That's it really")]
#[example(
  "This is your header!
This is the body of your article.
It's optional. But if you want it, you can have it"
)]
#[description("Create a Safety Briefing!")]
#[usage(
  "your briefing heading (one line)
the body of your heading (optional).
second line and on"
)]
/// Allows you to create a single Safety Briefing
///
/// Your first line should be the title, and any lines after (if included)
/// after are the body of your article.
///
/// Once you submit, you will be asked to confirm your message.
/// Press the checkbox to confirm, and your briefing will be queued up
/// Briefings are sent on Mondays at 8:30 AM Eastern
pub async fn briefing(ctx: &Context, original_msg: &Message, args: Args) -> CommandResult {
  let mut text: Vec<&str> = args.message().split("\n").collect();
  let heading = text[0];

  let body = if text.len() > 1 {
    text.remove(0);

    Some(text.join("\n"))
  } else {
    None
  };

  let guild_id = {
    ctx
      .data
      .read()
      .await
      .get::<BriefingGuildKey>()
      .expect("Expected guild id")
      .clone()
  };

  if let Err(_) = Guild::get(&ctx, guild_id)
    .await?
    .member(&ctx, original_msg.author.id)
    .await
  {
    original_msg.reply(ctx, "You are not approved").await?;
    return Ok(());
  }

  let preview = format!(
    concat!(
      "The following is what people will see.\n",
      "If you approve, please press ✅ within 60 seconds.\n",
      "You can cancel with ❌\n\n",
      ">>> Heading: {}\n",
      "Body: {}"
    ),
    heading,
    match body {
      Some(ref text) => text,
      None => "None",
    }
  );

  let msg = original_msg.reply(ctx, preview).await?;

  for reaction in BRIEFING_REPLIES {
    msg
      .react(ctx, ReactionType::Unicode(reaction.to_string()))
      .await?;
  }

  let accepted = if let Some(reaction) = msg
    .await_reaction(&ctx)
    .timeout(Duration::from_secs(60))
    .await
  {
    let emoji = &reaction.as_inner_ref().emoji;

    emoji.as_data().as_str() == "✅"
  } else {
    false
  };

  let now = Utc::now().with_timezone(&EST5EDT);

  if accepted {
    let briefing = Briefing {
      author: original_msg.author.mention().to_string(),
      heading: heading.to_string(),
      body: body,
      time: now.format("%A %B %-d, %Y %-I:%M %P").to_string(),
    };

    let serialized = match bincode::serialize(&briefing) {
      Ok(data) => data,
      Err(_) => {
        msg
          .reply(
            ctx,
            "Could not serialize your briefing. Please but bot manager",
          )
          .await?;
        return Ok(());
      }
    };

    let result = {
      let lock = {
        let mut context = ctx.data.write().await;
        context
          .get_mut::<RedisConnectionKey>()
          .expect("Expected redis connection")
          .clone()
      };

      let mut redis_client = lock.lock().await;
      redis_client
        .0
        .zadd::<&str, i64, Vec<u8>, u64>(BRIEFING_KEY, serialized, now.timestamp())
        .await
    };

    match result {
      Ok(_) => msg.reply(ctx, "Your briefing was accepted").await?,
      Err(error) => {
        msg
          .reply(ctx, format!("Could not save your briefing: {:?}", error))
          .await?
      }
    };
  } else {
    msg.reply(ctx, "Your briefing was not submitted").await?;
  }

  Ok(())
}

pub async fn send_briefing(
  briefings: &MyVec,
  channel_id: u64,
  http: &Arc<Http>,
) -> Result<(), String> {
  let mut messages: Vec<String> = vec![];
  let mut current_message =
    String::from("*DOOTDOOTDOOTDOOT*. It's time for your Safety Weekly Briefing!\n\n");

  for briefing in &briefings.v {
    let brief: Briefing = match bincode::deserialize(&briefing) {
      Ok(data) => data,
      Err(error) => return Err(format!("Could not deserialize briefing: {:?}", error)),
    };

    let next_message = match brief.body {
      Some(ref body) => format!(
        concat!("**{}**\n", "```{}```", " - {} {}\n\n",),
        brief.heading, body, brief.author, brief.time
      ),
      None => format!(
        concat!("**{}**\n", " - {} {}\n\n"),
        brief.heading, brief.author, brief.time
      ),
    };

    if current_message.len() + next_message.len() < 2000 {
      current_message += &next_message;
    } else {
      messages.push(current_message);
      current_message = next_message;
    }
  }

  messages.push(current_message);
  let channel = ChannelId(channel_id);

  for message in messages {
    if let Err(error) = channel.say(http, &message).await {
      return Err(format!("Could not send briefing: {:?}", error));
    }
  }

  Ok(())
}
