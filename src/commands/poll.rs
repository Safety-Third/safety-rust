use chrono::{Duration, Utc};
use chrono_tz::{EST5EDT};
use lazy_static::lazy_static;
use regex::Regex;
use serde_json::{Value, json};
use serenity::{
  model::prelude::interactions::{
    application_command::*,
    message_component::{ButtonStyle, MessageComponentInteraction}
  },
  model::prelude::*, prelude::*, utils::Colour,
};

use crate::util::scheduler::{EMOJI_ORDER, Callable, Poll, RedisSchedulerKey};
use super::{
  util::{format_duration, get_str_or_error, get_user}
};

pub fn poll_command() -> Value {
  let mut options: Vec<Value> = vec![
    json!({
      "type": ApplicationCommandOptionType::String,
      "name": "topic",
      "description": "The topic of this poll",
      "required": true
    }),
    json!({
      "type": ApplicationCommandOptionType::String,
      "name": "time",
      "description": "In the form XdXhXmXs (3d2h1m1s 3 day, 2 hour, 1 minute, 1 sec; 2m30s 2 minute, 30 second)",
      "required": true
    }),

  ];

  for idx in 1..=20 {
    options.push(json!({
      "type": ApplicationCommandOptionType::String,
      "name": format!("option-{}", idx),
      "description": format!("poll option {}", idx),
      "required": idx < 3
    }))
  }

  return json!({
    "name": "poll",
    "description": "Creates an emoji-based poll for a certain topic.",
    "options": options
  })
}

pub async fn interaction_poll(ctx: &Context,
  interaction: &ApplicationCommandInteraction) -> Result<(), String> {

  let data = &interaction.data;

  if data.options.len() < 4 {
    return Err(String::from("You must have a topic, date, and at least two options"))
  }

  let user = get_user(&interaction);
  let mention = user.mention().to_string();
  let author_id = user.id.0;

  let channel = interaction.channel_id;

  let duration = {
    let date_str = get_str_or_error(&data.options[1].value, "You must provide a time")?;
    parse_time(&date_str.trim())?
  };

  let topic_str = get_str_or_error(&data.options[0].value, "You must provide a topic")?;
  let mut options: Vec<String> = vec![];

  for option in &data.options[2..] {
    if let Some(op) = &option.value {
      if let Some(op_str) = op.as_str() {
        options.push(String::from(op_str.trim()));
        continue;
      }
    }

    return Err(format!("Error parsing field {}. The value was {:?}", option.name, option.value))
  }

  let lock = {
    let mut context = ctx.data.write().await;
    context.get_mut::<RedisSchedulerKey>()
      .expect("Expected redis scheduler")
      .clone()
  };

  let time = (Utc::now() + duration).with_timezone(&EST5EDT);

  let reactions: Vec<ReactionType> = EMOJI_ORDER[0..options.len()]
    .iter()
    .map(|emoji| ReactionType::Unicode(emoji.to_string()))
    .collect();

  let poll_id = {
    let mut redis_scheduler = lock.lock().await;
    match redis_scheduler.reserve_id().await {
      Ok(id) => id,
      Err(error) => return Err(error.to_string())
    }
  };

  if let Err(error) = interaction.create_interaction_response(ctx, |resp| {
    resp.kind(InteractionResponseType::ChannelMessageWithSource)
    .interaction_response_data(|msg| msg
      .content(format!("Poll: '{}' by {}", &topic_str, &mention))
      .create_embed(|e| {
        let mut description = String::from(">>> ");

        for (count, option) in options.iter().enumerate() {
          description += &format!("{}. {}\n", count + 1, &option);
        }

        let time_str = time.format("%D %r %Z");

        e
          .colour(Colour::BLITZ_BLUE)
          .title(format!("Poll: {}", &topic_str))
          .field("duration", format_duration(&duration), true)
          .field("poll id", &poll_id, true)
          .field("ends at", time_str, false)
          .description(description)
      })
      .components(|comp| comp.create_action_row(|row|
        row.create_button(|button| button
            .style(ButtonStyle::Danger)
            .label("Delete this poll")
            .custom_id("delete")
        )
          .create_button(|button| button
            .style(ButtonStyle::Secondary)
            .label("Close this poll")
            .custom_id("close"))
      ))
    )

  }).await {
    return Err(error.to_string())
  }

  let message = match interaction.get_interaction_response(ctx).await {
    Ok(msg) => msg,
    Err(error) => return Err(error.to_string())
  };

  for react in reactions {
    let _ = message.react(ctx, react).await;
  }

  let poll = Poll {
    author: author_id,
    channel: channel.0,
    message: message.id.0,
    topic: topic_str
  };

  {
    let mut redis_scheduler = lock.lock().await;
    match redis_scheduler.schedule_job(&poll, &poll_id, time.timestamp(), duration.num_seconds()).await {
      Ok(_) => Ok(()),
      Err(error) => Err(error.to_string())
    }
  }
}

pub fn add_poll_command() -> Value {
  let mut options: Vec<Value> = vec![
    json!({
      "type": ApplicationCommandOptionType::String,
      "name": "poll_id",
      "description": "The id of the poll you wish to edit",
      "required": true
    }),
  ];

  for idx in 1..=18 {
    options.push(json!({
      "type": ApplicationCommandOptionType::String,
      "name": format!("option-{}", idx),
      "description": format!("poll option {}", idx),
      "required": idx == 1
    }))
  }

  return json!({
    "name": "poll",
    "description": "Creates an emoji-based poll for a certain topic.",
    "options": options
  })
}

pub async fn interaction_add_poll(ctx: &Context,
  interaction: &ApplicationCommandInteraction) -> Result<(), String> {

  let data = &interaction.data;

  if data.options.len() < 2 {
    return Err(String::from("You must have a poll ID and at least one new option"))
  }


  Ok(())
}

pub async fn handle_poll_interaction(ctx: &Context,
  interaction: &MessageComponentInteraction) -> Result<(), String> {

  let msg = &interaction.message;

  // are you the author?
  if msg.mentions.len() == 1 && interaction.user == msg.mentions[0] {
    match interaction.data.custom_id.as_str() {
      "close" => {
        let poll = {
          let lock = {
            let mut context = ctx.data.write().await;
            context.get_mut::<RedisSchedulerKey>()
              .expect("Expected redis instance")
              .clone()
          };

          let mut redis_scheduler = lock.lock().await;
          redis_scheduler.pop_job(&msg.embeds[0].fields[1].value).await
        };

        match poll {
          Ok(result) => {
            let _ = interaction.create_interaction_response(ctx, |resp| {
              resp.kind(InteractionResponseType::UpdateMessage)
                .interaction_response_data(|resp| resp.components(|comp| comp))
            }).await;

            if let Some(real_poll) = result {
              real_poll.call(&ctx.http).await;
            }

            Ok(())
          },
          Err(error) => Err(format!("An error occurred when trying to close the poll: {}", error))
        }
      },
      "delete" => {
        if let Err(error) = msg.delete(ctx).await {
          Err(format!("Could not delete poll: {}", error))
        } else {
          let lock = {
            let mut context = ctx.data.write().await;
            context.get_mut::<RedisSchedulerKey>()
              .expect("Expected redis instance")
              .clone()
          };

          {
            let mut redis_scheduler = lock.lock().await;
            let _ = redis_scheduler.remove_job(&msg.embeds[0].fields[1].value).await;
          };

          Ok(())
        }
      },
      _ => Ok(())
    }
  } else {
    let _ = interaction.create_interaction_response(ctx, |resp| {
      resp.kind(InteractionResponseType::DeferredUpdateMessage)
    }).await;

    Ok(())
  }
}



/// Converts a potential "timing string" (day, hour, minute, second) to a Duration
///
/// # Arguments
/// * `timing` - A potential timing string. A successful format would be
/// in the form (\d+d)?(\d+h)?(\d+m)?(\d+s?), or a single number (minutes).
/// This time string **must** be at least 30 seconds
/// # Returns
/// - `Err`: if the string is malformed, or less than 30 seconds
/// - `Ok`: a duration representing the amount of time for the `timing` string
fn parse_time(timing: &str) -> Result<Duration, &str> {
  lazy_static! {
    static ref MIN_DURATION: Duration = Duration::seconds(30);

    static ref RE: Regex = Regex::new(r"(?x)
      (?P<days>\d+d)?
      (?P<hours>\d+h)?
      (?P<minutes>\d+m)?
      (?P<seconds>\d+s)?$").unwrap();
  }

  if let Ok(time_in_minutes) = timing.parse::<i64>() {
    return Ok(Duration::minutes(time_in_minutes));
  }

  let caps = match RE.captures(timing) {
    Some(captures) => captures,
    None =>
      return Err("Invalid format. Should be of the form `(\\d+d)?(\\d+h)?(\\d+m?)?` Or a number (for seconds")
  };

  let mut duration = Duration::zero();

  if let Some(days) = caps.name("days") {
    let days_str = days.as_str();

    match days_str[..days_str.len() - 1].parse::<i64>() {
      Ok(days_int) => duration = duration + Duration::days(days_int),
      Err(_) => return Err("Must provide a numeric value for days")
    }
  }

  if let Some(hours) = caps.name("hours") {
    let hours_str = hours.as_str();

    match hours_str[..hours_str.len() - 1].parse::<i64>() {
      Ok(hours_int) => duration = duration + Duration::hours(hours_int),
      Err(_) => return Err("Must provide a numeric value for hours")
    }
  }

  if let Some(minutes) = caps.name("minutes") {
    match minutes.as_str().replace('m', "").parse::<i64>() {
      Ok(minutes_int) => duration = duration + Duration::minutes(minutes_int),
      Err(_) => return Err("Must provide a numeric value for minutes")
    }
  }

  if let Some(seconds) = caps.name("seconds") {
    let seconds_str = seconds.as_str();

    match seconds_str[..seconds_str.len() - 1].parse::<i64>() {
      Ok(seconds_int) => duration = duration + Duration::seconds(seconds_int),
      Err(_) => return Err("Must provide a numeric value for seconds")
    }
  }

  if duration < *MIN_DURATION {
    return Err("Poll must be at least 30 seconds");
  }

  Ok(duration)
}
