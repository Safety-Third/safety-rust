use chrono::{Duration, Utc};
use chrono_tz::{EST5EDT};
use lazy_static::lazy_static;
use regex::Regex;
use serenity::prelude::*;
use serenity::model::prelude::*;
use serenity::utils::Colour;
use serenity::framework::standard::{
  Args, CommandError, CommandResult,
  macros::command
};

use super::{ 
  types::{EMOJI_ORDER, Poll, RedisSchedulerKey, Task},
  util::format_duration
};

const MAX_POLL_ARGS: usize = 20;

#[command]
#[min_args(3)]
#[usage("topic time options_list")]
#[example("\"What are birds?\" 2d3h1m2s \":jeff:\" \"We don't know\"")]
/// Creates an emoji-based poll for a certain topic.
/// **NOTE**: It is important that statements involving multiple words are quoted if you want them to be together.
///
/// Correct poll:
/// `>poll "What are birds?" 2d3h1m2s ":jeff:" "We don't know"`
/// (two options, ":jeff:" and "We don't know")
/// Create a poll for 2 days, 3 hours, 1 minute and 2 seconds from now
///    
/// Incorrect poll:
/// `>poll "What are birds?" 2d3h1m ":jeff:" We don't know`
/// (four options, ":jeff:", "We", "don't", and "know")
///
/// When providing times, here is the general format: XdXhXmXs. Replace X with a number. Examples:
/// - 1d (1 day)
/// - 1d3h10m35s (1 day, 3 hours, 10 minutes, 35s)
/// - 3h5m (3 hours, 5 minutes)
/// - 5m (5 minutes)
/// - 5 (5 minutes)
pub fn poll(ctx: &mut Context, msg: &Message, mut args: Args) -> CommandResult {
  let topic: String = args.single_quoted()?;
  let timing: String = args.single_quoted()?;
  
  let duration = parse_time(&timing)?;

  if args.remaining() > MAX_POLL_ARGS {
    return command_err_str!("You can have a maximum of 20 options");
  } else if args.is_empty() {
    return command_err_str!("You must have at least one option");
  }

  let mut options: Vec<String> = vec![];

  for _ in 0..args.remaining() {
    options.push(args.single_quoted()?);
  }

  let lock = {
    let mut context = ctx.data.write();
    context.get_mut::<RedisSchedulerKey>()
      .expect("Expected redis scheduler")
      .clone()
  };

  let time = (Utc::now() + duration).with_timezone(&EST5EDT);

  let reactions: Vec<String> = EMOJI_ORDER[0..options.len()]
    .iter()
    .map(|emoji| emoji.to_string())
    .collect();

  let message = msg.channel_id.send_message(&ctx.http, |m| {
    m
      .content(format!{"Poll: '{}' by {}", &topic, msg.author.mention()})
      .embed(|e| {
        let mut description = String::from(">>> ");

        for (count, option) in options.iter().enumerate() {
          description += &format!("{}. {}\n", count + 1, &option);
        }

        let time_str = time.format("%D %r %Z");

        e
          .colour(Colour::BLITZ_BLUE)
          .title(format!("Poll: {}", &topic))
          .field("author", msg.author.mention(), true)
          .field("duration", format_duration(&duration), true)
          .field("ends at", time_str, false)
          .description(description)
      })
      .reactions(reactions)
  })?;

  let poll = Poll {
    author: msg.author.id.0,
    channel: msg.channel_id.0,
    message: message.id.0,
    topic
  };

  {
    let mut redis_scheduler = lock.lock();
    redis_scheduler.schedule_job(&Task::Poll(poll), time.timestamp())?
  };
  
  Ok(())
}

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




