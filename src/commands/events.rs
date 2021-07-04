use chrono::{Duration, Utc};
use serde_json::{Value, json};
use serenity::prelude::*;
use serenity::model::prelude::*;
use serenity::utils::Colour;
use serenity::framework::standard::{
  Args, CommandResult, macros::command
};

use super::{
  types::{Event, RedisSchedulerKey, Task},
  util::{format_duration, get_str_or_error, get_user, handle_command_err}
};
use crate::util::datetime_parse::*;

pub fn cancel_command() -> Value {
  json!({
    "name": "cancel",
    "description": "Cancels an event that you have scheduled",
    "options": [{
      "type": ApplicationCommandOptionType::String,
      "name": "event_id",
      "description": "The id of the event to cancel",
      "required": true
    }]
  })
}

pub async fn interaction_cancel(ctx: &Context, interaction: &Interaction,
  data: &ApplicationCommandInteractionData) -> Result<(), String> {

  if data.options.len() == 0 {
    return Err(String::from("You must provide an id"))
  }

  let event_id = get_str_or_error(&data.options[0].value, "You must provide an id")?;
  let author = get_user(&interaction)?;

  let event = cancel_event(&ctx, &author, &event_id).await?;

  let mut message = format!("Cancelled event {} at {} by {}",
  event.event, event.time, UserId(event.author).mention());

  if !event.members.is_empty() {
    message += "\n";
    message += &event.members
      .iter()
      .map(|member| UserId(*member).mention().to_string())
      .collect::<Vec<String>>()
      .join(", ");
  }
  
  let channel_id = interaction.channel_id.unwrap_or(ChannelId(0));

  if channel_id.0 == event.channel {
    if let Err(error) = interaction.create_interaction_response(&ctx.http, |resp| {
      resp.kind(InteractionResponseType::ChannelMessageWithSource)
        .interaction_response_data(|msg| msg.content(&message))
    }).await {
      return Err(error.to_string())
    }

  } else {
    let broadcast_res = match ChannelId(event.channel).say(&ctx.http, &message).await {
      Ok(_) => format!("You cancelled event {} for {}", event.event, event.time),
      Err(error) => format!(
        "You cancelled event {} for {}, but there was an error sending notifications: {:?}",
        event.event, event.time, error)
    };

    if let Err(error) = interaction.create_interaction_response(&ctx.http, |resp| {
      resp.kind(InteractionResponseType::ChannelMessageWithSource)
        .interaction_response_data(|msg| msg.content(&broadcast_res))
    }).await {
      return Err(error.to_string())
    }
  }

  Ok(())
}

#[command]
#[num_args(1)]
#[usage("event_id")]
#[example("00000000000000000000000000000000")]
/// Cancels an event that you have scheduled.
/// You must be the creator of an event to cancel it.
/// This will notify members in the channel that you have cancelled the event.
pub async fn cancel(ctx: &Context, msg: &Message, mut args: Args) -> CommandResult {
  let job_id: String = args.single_quoted()?;

  let event = match cancel_event(&ctx, &msg.author, &job_id).await {
    Ok(ev) => ev,
    Err(error) => return handle_command_err(ctx, msg, &error).await
  };

  let mut message = format!("Cancelled event {} at {} by {}",
    event.event, event.time, UserId(event.author).mention());

  if !event.members.is_empty() {
    message += "\n";
    message += &event.members
      .iter()
      .map(|member| UserId(*member).mention().to_string())
      .collect::<Vec<String>>()
      .join(", ");
  }

  ChannelId(event.channel).say(&ctx.http, &message).await?;

  if msg.channel_id.0 != event.channel {
    let _ = msg.channel_id.say(&ctx.http,
        format!("You cancelled event {} for {}",
          event.event, event.time)).await;
  }

  Ok(())
}

async fn cancel_event(ctx: &Context, author: &User, job_id: &str) -> Result<Event, String> {
  let lock = {
    let mut context = ctx.data.write().await;
    context.get_mut::<RedisSchedulerKey>()
      .expect("Expected redis scheduler")
      .clone()
  };

  let mut redis_scheduler = lock.lock().await;
  let existing_event = match redis_scheduler.get_job(&job_id) {
    Ok(res) => res,
    Err(error) => return Err(error.to_string())
  };

  let existing_event = match existing_event {
    Task::Event(item) => item,
    Task::Poll(_) => return Err(format!("No event {} found", &job_id))
  };

  if existing_event.author != author.id.0 {
    return Err(String::from("You are not the owner of this event"));
  }

  if let Err(error) = redis_scheduler.remove_job(&job_id) {
    return Err(error.to_string());
  }

  Ok(existing_event)
}

async fn user_action(ctx: &Context, interaction: &Interaction,
  data: &ApplicationCommandInteractionData, is_leave: bool, success_msg: &str, 
  fail_msg: &str) -> Result<(), String> {

  if data.options.len() == 0 {
    return Err(String::from("You must provide an id"))
  }
  
  let event_id = get_str_or_error(&data.options[0].value, "You must provide an id")?;
  let msg_author = get_user(&interaction)?;
  let channel_id = interaction.channel_id.unwrap_or(ChannelId(0));

  let (changed, event_id, topic, author) = if is_leave {
    leave_event(&ctx, &msg_author, &event_id).await?
  } else {
    signup_event(&ctx, &msg_author, &event_id).await?
  };

  if changed {
    let channel_msg = format!("{} {} \"{}\" by {}",
      msg_author.mention(), success_msg, topic, UserId(author).mention());

    if channel_id.0 == event_id {
      let _ = interaction.create_interaction_response(&ctx.http, |resp| {
        resp.kind(InteractionResponseType::ChannelMessageWithSource)
          .interaction_response_data(|msg| msg.content(channel_msg))
      }).await;
    } else {
      let _ = ChannelId(event_id).say(&ctx.http, &channel_msg).await;

      let _ = interaction.create_interaction_response(&ctx.http, |resp| {
        resp.kind(InteractionResponseType::ChannelMessageWithSource)
          .interaction_response_data(|msg| msg.content("Done!"))
      }).await;
    }
  } else {
    let final_msg = format!("{} {}", fail_msg, event_id);

    let _ = interaction.create_interaction_response(&ctx.http, |response| {
      response.kind(InteractionResponseType::ChannelMessageWithSource)
      .interaction_response_data(|message| message.content(final_msg))
    }).await;
  }

  Ok(())
}

pub fn leave_command() -> Value {
  json!({
    "name": "leave",
    "description": "Leave an event that you have signed up for",
    "options": [{
      "type": ApplicationCommandOptionType::String,
      "name": "event_id",
      "description": "The id of the event to leave",
      "required": true
    }]
  })
}

pub async fn interaction_leave(ctx: &Context, interaction: &Interaction,
  data: &ApplicationCommandInteractionData) -> Result<(), String> {

  user_action(ctx, interaction, data, true, "left event", "You were not in event").await
}

#[command]
#[num_args(1)]
#[usage("event_id")]
#[example("00000000000000000000000000000000")]
/// Leave an event that you have signed up for
/// This will ntoify members of the channel that you are no longer planning
/// to attend the event. If you are not signed up for the event, 
/// no message will be sent
pub async fn leave(ctx: &Context, msg: &Message, mut args: Args) -> CommandResult {
  let job_id: String = args.single_quoted()?;

  let (changed, channel_id, topic, author) = match leave_event(ctx, &msg.author, &job_id).await {
    Ok(res) => res,
    Err(error) => return handle_command_err(ctx, msg, &error).await 
  };

  if changed {
    ChannelId(channel_id).say(&ctx.http,
      format!("{} left event \"{}\" by {}", 
        msg.author.mention(), topic, UserId(author).mention())).await?;
  }

  Ok(())
}

async fn leave_event(ctx: &Context, msg_author: &User, job_id: &str) -> 
  Result<(bool, u64, String, u64), String> {
  let lock = {
    let mut context = ctx.data.write().await;
    context.get_mut::<RedisSchedulerKey>()
      .expect("Expected redis scheduler")
      .clone()
  };

  let mut redis_scheduler = lock.lock().await;
  let existing_event = match redis_scheduler.get_job(&job_id) {
    Ok(res) => res,
    Err(error) => return Err(error.to_string())
    
  };

  let mut existing_event = match existing_event {
    Task::Event(item) => item,
    Task::Poll(_) => return Err(format!("No event {} found", &job_id))
  };

  let author = existing_event.author;
  let id = existing_event.channel;
  
  if existing_event.members.contains(&msg_author.id.0) {
    let topic = existing_event.event.clone();

    existing_event.members.retain(|member| *member != msg_author.id.0);
    if let Err(error) = redis_scheduler.edit_job(&Task::Event(existing_event), &job_id, None) {
      return Err(error.to_string());
    }
    Ok((true, id, topic, author))
  } else {
    Ok((false, 0, String::new(), author))
  }
}

pub fn reschedule_command() -> Value {
  json!({
    "name": "reschedule",
    "description": "Reschedules an event to another time (only the creator)",
    "options": [{
      "type": ApplicationCommandOptionType::String,
      "name": "event_id",
      "description": "The id of the event to reschedule",
      "required": true
    }, {
      "type": ApplicationCommandOptionType::String,
      "name": "date",
      "description": "When to reschedule, format mm/dd/yy hh:mm AM/PM tz (1/1/11 2:01 PM EDT). >help for more options"
    }]
  })
}

pub async fn interaction_reschedule(ctx: &Context, interaction: &Interaction,
  data: &ApplicationCommandInteractionData) -> Result<(), String> {

  if data.options.len() < 2 {
    return Err(String::from("You must provide the event id and time"))
  }

  let event_id = get_str_or_error(&data.options[0].value, "You must provide an id")?;
  let new_time = get_str_or_error(&data.options[1].value, "you must provide a date")?;

  let event_time = parse_datetime_tz(&new_time)?;
  let now = Utc::now();

  if event_time + Duration::minutes(1) < now {
    return Err(String::from("Event must be at least one minute in the future"))
  }

  let lock = {
    let mut context = ctx.data.write().await;
    context.get_mut::<RedisSchedulerKey>()
      .expect("Expected redis scheduler")
      .clone()
  };

  let existing_job = {
    let mut scheduler = lock.lock().await;
    scheduler.get_job(&event_id)
  };

  let existing_job = match existing_job {
    Ok(task) => task,
    Err(_) => return Err(format!("Could not find an event {}", event_id))
  };

  let msg_author = get_user(&interaction)?;
  let channel_id = interaction.channel_id.unwrap_or(ChannelId(0));

  if let Task::Event(mut event) = existing_job {
    if event.author != msg_author.id.0 {
      return Err(String::from("You must be the creator to reschedule this event"));
    }

    event.time = new_time.clone();

    {
      let channel = ChannelId(event.channel);
      let event_name = event.event.clone();
      let members = event.members_and_author();

      let has_two_messages = channel_id.0 == event.channel;

      if let Err(fail) = {
        let mut scheduler = lock.lock().await;
        scheduler.edit_job(&Task::Event(event), &event_id, Some(event_time.timestamp()))
      } {
        return Err(fail.to_string());
      };

      let reschedule_msg = format!("Rescheduled \"{}\" to {}: {}", event_name, new_time, members);

      if has_two_messages {
        let _ = interaction.create_interaction_response(&ctx.http, |resp| {
          resp.kind(InteractionResponseType::ChannelMessageWithSource)
            .interaction_response_data(|msg| msg.content(reschedule_msg))
        }).await;
      } else {
        let _ = channel.say(&ctx.http, reschedule_msg).await;

        let _ = interaction.create_interaction_response(&ctx.http, |resp| {
          resp.kind(InteractionResponseType::ChannelMessageWithSource)
            .interaction_response_data(|msg| msg.content("Done!"))
        }).await;
      }
    }
    Ok(())
  } else {
    Err(format!("Could not find an event {}", event_id))
  }
}

#[command]
#[num_args(2)]
#[usage("event_id new_time")]
#[example("00000000000000000000000000000000 \"3/27/20 15:39 EDT\"")]
/// Reschedules an event to another time.
/// This will notify all individuals who have signed up in the original channel
/// You must be the creater of this event to reschedule it
pub async fn reschedule(ctx: &Context, msg: &Message, mut args: Args) -> CommandResult {
  let job_id: String = args.single_quoted()?;
  let new_time: String = args.single_quoted()?;
  
  let event_time = parse_datetime_tz(&new_time)?;
  let now = Utc::now();

  if event_time + Duration::minutes(1) < now {
    return handle_command_err(ctx, msg, "Event must be at least one minute in the future").await;
  }

  let lock = {
    let mut context = ctx.data.write().await;
    context.get_mut::<RedisSchedulerKey>()
      .expect("Expected redis scheduler")
      .clone()
  };

  let existing_job = {
    let mut scheduler = lock.lock().await;
    scheduler.get_job(&job_id)
  };
  
  let existing_job = match existing_job {
    Ok(task) => task,
    Err(_) => return command_err!(format!("Could not find an event {}", job_id))
  };
  
  if let Task::Event(mut event) = existing_job {
    if event.author != msg.author.id.0 {
      return command_err!("You must be the creator to reschedule this event");
    }

    event.time = new_time.clone();

    {
      let channel = ChannelId(event.channel);
      let event_name = event.event.clone();
      let members = event.members_and_author();

      if let Err(fail) = {
        let mut scheduler = lock.lock().await;
        scheduler.edit_job(&Task::Event(event), &job_id, Some(event_time.timestamp()))
      } {
        return command_err!(fail.to_string());
      };

      let _ = channel.say(&ctx.http,
        format!("Rescheduled \"{}\" to {}: {}", event_name, new_time, members)).await;

    }
    Ok(())
  } else {
    command_err!(format!("Could not find an event {}", job_id))
  }
}

pub fn schedule_command() -> Value {
  return json!({
    "name": "schedule",
    "description": "Schedules an event",
    "options": [{
      "type": ApplicationCommandOptionType::String,
      "name": "topic",
      "description": "The topic of the event",
      "required": true
    }, {
      "type": ApplicationCommandOptionType::String,
      "name": "date",
      "description": "When to have the event, format mm/dd/yy hh:mm AM/PM tz (1/1/11 2:01 PM EDT). >help for more options",
      "required": true
    }]
  })
}

pub async fn interaction_schedule(ctx: &Context, interaction: &Interaction,
  data: &ApplicationCommandInteractionData) -> Result<(), String> {

  if data.options.len() < 2 {
    return Err(String::from("You must provide the event topic and time"))
  }

  let event_name = get_str_or_error(&data.options[0].value, "You must provide a topic")?;
  let time = get_str_or_error(&data.options[1].value, "You must provide a time")?;
  let msg_author = get_user(&interaction)?;
  let channel_id = interaction.channel_id.unwrap_or(ChannelId(0));

  let event_time = parse_datetime_tz(&time)?;

  let now = Utc::now();

  if event_time + Duration::minutes(1) < now {
    return Err(String::from("Event must be at least one minute in the future"))
  }

  let duration: Duration = event_time.with_timezone(&Utc) - now;

  let event = Event {
    author: msg_author.id.0,
    channel: channel_id.0,
    event: String::from(&event_name),
    members: vec![],
    time
  };

  let lock = {
    let mut context = ctx.data.write().await;
    context.get_mut::<RedisSchedulerKey>()
      .expect("Expected redis scheduler")
      .clone()
  };

  let job_id = {
    let mut redis_scheduler = lock.lock().await;
    redis_scheduler.schedule_job(&Task::Event(event), event_time.timestamp())
  };
  
  let job_id = match job_id {
    Ok(job) => job,
    Err(error) => return Err(error.to_string())
  };

  if let Err(error) = interaction.create_interaction_response(&ctx.http, |resp| {
    let time_str = event_time.format("%D %r %Z");

    resp.kind(InteractionResponseType::ChannelMessageWithSource)
    .interaction_response_data(|msg| msg
      .content(format!("Event '{}' by {}", &event_name, msg_author.mention()))
      .create_embed(|e| {
        e
          .color(Colour::BLITZ_BLUE)
          .title(format!("Event: {}", &event_name))
          .field("scheduled for", time_str, false)
          .field("author", msg_author.mention(), true)
          .field("happening in", format_duration(&duration), true)
          .field("id", job_id, false)
      })
    )

  }).await {
    return Err(error.to_string())
  }

  Ok(())
}

#[command]
#[num_args(2)]
#[usage("\"event topic\" \"datetime string\"")]
#[example("\"A test event\" \"3/27/20 15:39 EDT\"")]
#[example("\"A test event\" \"3/27/20 3:39 PM EDT\"")]
#[example("\"A test event\" \"3/27/20 3:39 PM America/New_York\"")]
/// Schedules an event
/// This accepts the following formats:
/// - AM/PM, with seconds: mm/dd/yy hh:mm:ss AM/PM tz (1/1/11 1:11:11 AM EDT)
/// - AM/PM, no seconds: mm/dd/yy hh:mm AM/PM tz (1/1/11 2:01 pm CST)
/// - 24-hour, seconds: mm/dd/yy HH:mm:ss tz (1/1/13 13:13:13 UTC-4)
/// - 24-hour, no seconds: mm/dd/yy HH:mm tz (1/01/13 08:27 PST)
///
/// The following time zones have been provided:
/// - EDT, EST, CDT, CST, MDT, MST, PDT, PST, AKDT, AKST
/// For other time zones, please use the time zone name (e.g. America/New_York)
pub async fn schedule(ctx: &Context, msg: &Message, mut args: Args) -> CommandResult {
  let event_name: String = args.single_quoted()?;
  let time: String = args.single_quoted()?;

  let event_time = parse_datetime_tz(&time)?;

  let now = Utc::now();

  if event_time + Duration::minutes(1) < now {
    return handle_command_err(ctx, msg, "Event must be at least one minute in the future").await;
  }

  let duration: Duration = event_time.with_timezone(&Utc) - now;

  let event = Event {
    author: msg.author.id.0,
    channel: msg.channel_id.0,
    event: String::from(&event_name),
    members: vec![],
    time
  };

  let lock = {
    let mut context = ctx.data.write().await;
    context.get_mut::<RedisSchedulerKey>()
      .expect("Expected redis scheduler")
      .clone()
  };

  let job_id = {
    let mut redis_scheduler = lock.lock().await;
    redis_scheduler.schedule_job(&Task::Event(event), event_time.timestamp())
  };
  
  let job_id = match job_id {
    Ok(job) => job,
    Err(error) => return handle_command_err(ctx, msg, &error.to_string()).await
  };

  let _ = msg.channel_id.send_message(&ctx.http, |m| {
    let time_str = event_time.format("%D %r %Z");
    m
      .content(format!("Event '{}' by {}", &event_name, msg.author.mention()))
      .embed(|e| {
        e
          .color(Colour::BLITZ_BLUE)
          .title(format!("Event: {}", &event_name))
          .field("scheduled for", time_str, false)
          .field("author", msg.author.mention(), true)
          .field("happening in", format_duration(&duration), true)
          .field("id", job_id, false)
      })
  }).await;

  Ok(())
}

pub fn signup_command() -> Value {
  json!({
    "name": "signup",
    "description": "sign up for a scheduled event using an event ID",
    "options": [{
      "type": ApplicationCommandOptionType::String,
      "name": "event_id",
      "description": "The id of the event to leave",
      "required": true
    }]
  })
}

pub async fn interaction_signup(ctx: &Context, interaction: &Interaction,
  data: &ApplicationCommandInteractionData) -> Result<(), String> {

  user_action(ctx, interaction, data, false, "signed up for", "You have already signed up for event").await
}

#[command]   
#[num_args(1)]
#[usage("event_id")]
#[example("00000000000000000000000000000000")]
/// Signup for a scheduled event using an event ID.
/// You must be able to see the channel to sign up for the event.
/// Signing up for an event will notify other members in the channel
pub async fn signup(ctx: &Context, msg: &Message, mut args: Args) -> CommandResult {
  let job_id: String = args.single_quoted()?;

  let (changed, channel_id, topic, author) =  match signup_event(ctx, &msg.author, &job_id).await {
    Ok(res) => res,
    Err(error) => return handle_command_err(ctx, msg, &error).await
  };

  if changed {
    ChannelId(channel_id).say(&ctx.http,
      format!("{} signed up for \"{}\" by {}", 
        msg.author.mention(), topic, UserId(author).mention())).await?;
  }

  Ok(())
}

async fn signup_event(ctx: &Context, user: &User, job_id: &str) -> Result<(bool, u64, String, u64), String> {
  let lock = {
    let mut context = ctx.data.write().await;
    context.get_mut::<RedisSchedulerKey>()
      .expect("Expected redis scheduler")
      .clone()
  };

  let mut redis_scheduler = lock.lock().await;
  let existing_event = match redis_scheduler.get_job(&job_id) {
    Ok(event) => event,
    Err(error) => return Err(error.to_string())
  };

  let mut existing_event = match existing_event {
    Task::Event(item) => item,
    Task::Poll(_) => return Err(format!("No event {} found", &job_id))
  };

  let author = existing_event.author;
  let id = existing_event.channel;

  let channel = match ChannelId(id).to_channel_cached(&ctx.cache).await {
    Some(exists) => {
      match exists.guild() {
        Some(guild_channel) => guild_channel,
        None => return Err(format!("Channel {} not found", &id))
      }
    },
    None => return Err(format!("Channel {} not found", &id))
  };

  {
    let members = match channel.members(&ctx.cache).await {
      Ok(m) => m,
      Err(error) => return Err(error.to_string())
    };
    
    if !members.iter().any(|m| m.user.id == user.id) {
      return Err(format!("No event {} found", &job_id))
    }
  };
  
  if !existing_event.members.contains(&user.id.0) {
    let topic = existing_event.event.clone();

    existing_event.members.push(user.id.0);
    if let Err(error) = redis_scheduler.edit_job(&Task::Event(existing_event), &job_id, None) {
      return Err(error.to_string())
    }
    Ok((true, id, topic, author))
  } else {
    Ok((false, 0, String::new(), author))
  }
}