use chrono::{Duration, Utc};
use chrono_tz::{EST5EDT};
use lazy_static::lazy_static;
use regex::{Match,Regex};
use serde_json::{Value, json};
use serenity::{
  framework::standard::{Args, CommandResult, macros::command},
  model::prelude::*, prelude::*, utils::Colour,
};

use super::{ 
  types::{EMOJI_ORDER, Poll, RedisSchedulerKey, Task},
  util::{format_duration, get_str_or_error, get_user}
};

const MAX_POLL_ARGS: usize = 20;

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

pub async fn interaction_poll(ctx: &Context, interaction: &Interaction,
  data: &ApplicationCommandInteractionData) -> Result<(), String> {
  lazy_static! {
    static ref RE: Regex = Regex::new(r"\[[^\[\]]+\]").unwrap();
  }

  if data.options.len() < 4 {
    return Err(String::from("You must have a topic, date, and at least two options"))
  }

  let user = get_user(&interaction)?;
  let mention = user.mention().to_string();
  let author_id = user.id.0;

  let channel = match interaction.channel_id {
    Some(c) => c,
    None => return Err(String::from("Must have channel id"))
  };

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

  if let Err(error) = interaction.create_interaction_response(&ctx.http, |resp| {
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
          .field("author", &mention, true)
          .field("duration", format_duration(&duration), true)
          .field("ends at", time_str, false)
          .description(description)
      })
    )

  }).await {
    return Err(error.to_string())
  }

  let message = match interaction.get_interaction_response(&ctx.http).await {
    Ok(msg) => msg,
    Err(error) => return Err(error.to_string())
  };

  for react in reactions {
    let _ = message.react(&ctx.http, react).await;
  }

  let poll = Poll {
    author: author_id, 
    channel: channel.0,
    message: message.id.0,
    topic: topic_str
  };

  {
    let mut redis_scheduler = lock.lock().await;
    match redis_scheduler.schedule_job(&Task::Poll(poll), time.timestamp()) {
      Ok(_) => Ok(()),
      Err(error) => Err(error.to_string())
    }
  }
}

#[command]
#[usage("time topic options_list")]
#[example("2d3h1m2s What are birds? [:jeff:] [We don't know]")]
#[example("2d3h1m2s What are birds?
:jeff:
We don't know
")]
/// Creates an emoji-based poll for a certain topic. Options Options can
/// either be provided surrounded by [], such as `[this is an option]`, or on
/// subsequent lines after the topic. **DO NOT** Mix both
///
/// When providing times, here is the general format: XdXhXmXs. Replace X with a number. Examples:
/// - 1d (1 day)
/// - 1d3h10m35s (1 day, 3 hours, 10 minutes, 35s)
/// - 3h5m (3 hours, 5 minutes)
/// - 5m (5 minutes)
/// - 5 (5 minutes)
pub async fn poll(ctx: &Context, msg: &Message, mut args: Args) -> CommandResult {
  lazy_static! {
    static ref RE: Regex = Regex::new(r"\[[^\[\]]+\]").unwrap();
  }

  let first: String = args.single_quoted()?;
  let remaining = match args.remains() {
    Some(string) => string,
    None => return error_with_usage(String::from("You must provide a poll topic and options"))
  };

  let mut options: Vec<&str>;

  let topic: &str;

  if remaining.contains("\n") {
    let lines: Vec<&str> = remaining.split("\n")
      .filter(|line| !line.trim().is_empty())
      .map(|line| line.trim())
      .collect();
    topic = lines[0];
    options = lines[1..].to_owned();
    
  } else {
    options = vec![];

    let matches: Vec<Match> = RE.find_iter(remaining).collect();
    
    if matches.len() == 0 {
      return error_with_usage(String::from("You must provide at least one option"));
    } else if matches.len() > MAX_POLL_ARGS {
      return error_with_usage(format!("You can have at maximum {} options", MAX_POLL_ARGS));
    }

    topic = &remaining[..matches[0].start() - 1].trim();
    
    for option in matches {
      options.push(&remaining[option.start() + 1 .. option.end() - 1].trim());
    }
  }

  let duration = parse_time(&first)?;

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
  }).await?;

  let poll = Poll {
    author: msg.author.id.0,
    channel: msg.channel_id.0,
    message: message.id.0,
    topic: topic.to_owned()
  };

  {
    let mut redis_scheduler = lock.lock().await;
    redis_scheduler.schedule_job(&Task::Poll(poll), time.timestamp())?
  };
  
  Ok(())
}

fn error_with_usage(base_err: String) -> CommandResult {
  return command_err!(format!("{}\nHere are two examples:
`>poll 1m this is my topic [option 1, in brackets] [option 2, also in brackets]`

`>poll 1m this is my topic
option 1, on a separate line
option 2, also on a separate line`
", base_err));
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
