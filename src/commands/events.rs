use chrono::{Duration, Utc};
use serenity::prelude::*;
use serenity::model::prelude::*;
use serenity::utils::Colour;
use serenity::framework::standard::{
  Args, CommandError, CommandResult,
  macros::command
};

use super::{
  types::{Event, RedisSchedulerKey, Task},
  util::format_duration
};
use crate::util::datetime_parse::*;

#[command]
#[num_args(1)]
#[usage("event_id")]
#[example("00000000000000000000000000000000")]
/// Cancels an event that you have scheduled.
/// You must be the creator of an event to cancel it.
/// This will notify members in the channel that you have cancelled the event.
pub fn cancel(ctx: &mut Context, msg: &Message, mut args: Args) -> CommandResult {
  let job_id: String = args.single_quoted()?;

  let lock = {
    let mut context = ctx.data.write();
    context.get_mut::<RedisSchedulerKey>()
      .expect("Expected redis scheduler")
      .clone()
  };

  let event = {
    let mut redis_scheduler = lock.lock();
    let existing_event = redis_scheduler.get_job(&job_id)?;

    let existing_event = match existing_event {
      Task::Event(item) => item,
      Task::Poll(_) => 
        return command_err!(format!("No event {} found", &job_id))
    };

    if existing_event.author != msg.author.id.0 {
      return command_err_str!("You are not the owner of this event");
    }

    redis_scheduler.remove_job(&job_id)?;

    existing_event
  };

  let mut message = format!("Cancelled event {} at {} by {}",
    event.event, event.time, UserId(event.author).mention());

  if !event.members.is_empty() {
    message += "\n";
    message += &event.members
      .iter()
      .map(|member| UserId(*member).mention())
      .collect::<Vec<String>>()
      .join(", ");
  }

  ChannelId(event.channel).say(&ctx.http, &message)?;

  if msg.channel_id.0 != event.channel {
    msg.channel_id.say(&ctx.http,
        format!("You cancelled event {} for {}",
          event.event, event.time))?;
  }

  Ok(())
}

#[command]
#[num_args(1)]
#[usage("event_id")]
#[example("00000000000000000000000000000000")]
/// Leave an event that you have signed up for
/// This will ntoify members of the channel that you are no longer planning
/// to attend the event. If you are not signed up for the event, 
/// no message will be sent
pub fn leave(ctx: &mut Context, msg: &Message, mut args: Args) -> CommandResult {
  let job_id: String = args.single_quoted()?;

  let lock = {
    let mut context = ctx.data.write();
    context.get_mut::<RedisSchedulerKey>()
      .expect("Expected redis scheduler")
      .clone()
  };

  let (changed, channel_id, topic, author) = {
    let mut redis_scheduler = lock.lock();
    let existing_event = redis_scheduler.get_job(&job_id)?;

    let mut existing_event = match existing_event {
      Task::Event(item) => item,
      Task::Poll(_) => 
        return command_err!(format!("No event {} found", &job_id))
    };

    let author = existing_event.author;
    let id = existing_event.channel;
    
    if existing_event.members.contains(&msg.author.id.0) {
      let topic = existing_event.event.clone();

      existing_event.members.retain(|member| *member != msg.author.id.0);
      redis_scheduler.edit_job(&Task::Event(existing_event), &job_id, None)?;
      (true, id, topic, author)
    } else {
      (false, 0, String::new(), author)
    }
  };

  if changed {
    ChannelId(channel_id).say(&ctx.http,
      format!("{} left event \"{}\" by {}", 
        msg.author.mention(), topic, UserId(author).mention()))?;
  }

  Ok(())
}

#[command]
#[num_args(2)]
#[usage("event_id new_time")]
#[example("00000000000000000000000000000000 \"3/27/20 15:39 EDT\"")]
/// Rescedules an event to another time.
/// This will notify all individuals who have signed up in the original channel
/// You must be the creater of this event to reschedule it
pub fn reschedule(ctx: &mut Context, msg: &Message, mut args: Args) -> CommandResult {
  let job_id: String = args.single_quoted()?;
  let new_time: String = args.single_quoted()?;
  
  let event_time = parse_datetime_tz(&new_time)?;
  let now = Utc::now();

  if event_time + Duration::minutes(1) < now {
    return Err(CommandError(String::from("Event must be at least one minute in the future")));
  }

  let lock = {
    let mut context = ctx.data.write();
    context.get_mut::<RedisSchedulerKey>()
      .expect("Expected redis scheduler")
      .clone()
  };

  let existing_job = {
    let mut scheduler = lock.lock();
    match scheduler.get_job(&job_id) {
      Ok(task) => task,
      Err(_) => return command_err!(format!("Could not find an event {}", job_id))
    }
  };
  
  if let Task::Event(mut event) = existing_job {
    if event.author != msg.author.id.0 {
      return command_err_str!("You must be the creator to reschedule an event");
    }

    event.time = new_time.clone();

    {
      let channel = ChannelId(event.channel);
      let event_name = event.event.clone();
      let members = event.members_and_author();

      let mut scheduler = lock.lock();
      if let Err(fail) = scheduler.edit_job(&Task::Event(event), 
        &job_id, Some(event_time.timestamp())) {
        return command_err!(fail.to_string());
      } else if let Err(error) = channel.say(&ctx.http, 
          format!("Rescheduled \"{}\" to {}: {}", event_name, new_time, members)) {
          
        let _ = msg.author.dm(&ctx.http, |m| {
          m.content(format!("Could not rescueduel {}: {}", event_name, error))
        });
      }
      
    };
    Ok(())
  } else {
    command_err!(format!("Could not find an event {}", job_id))
  }
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
pub fn schedule(ctx: &mut Context, msg: &Message, mut args: Args) -> CommandResult {
  let event_name: String = args.single_quoted()?;
  let time: String = args.single_quoted()?;

  let event_time = parse_datetime_tz(&time)?;

  let now = Utc::now();

  if event_time + Duration::minutes(1) < now {
    return Err(CommandError(String::from("Event must be at least one minute in the future")));
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
    let mut context = ctx.data.write();
    context.get_mut::<RedisSchedulerKey>()
      .expect("Expected redis scheduler")
      .clone()
  };

  let job_id = {
    let mut redis_scheduler = lock.lock();
    redis_scheduler.schedule_job(&Task::Event(event), event_time.timestamp())?
  };

  msg.channel_id.send_message(&ctx.http, |m| {
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
  })?;

  Ok(())
}

#[command]
#[num_args(1)]
#[usage("event_id")]
#[example("00000000000000000000000000000000")]
/// Signup for a scheduled event using an event ID.
/// You must be able to see the channel to sign up for the event.
/// Signing up for an event will notify other members in the channel
pub fn signup(ctx: &mut Context, msg: &Message, mut args: Args) -> CommandResult {
  let job_id: String = args.single_quoted()?;

  let lock = {
    let mut context = ctx.data.write();
    context.get_mut::<RedisSchedulerKey>()
      .expect("Expected redis scheduler")
      .clone()
  };

  let (changed, channel_id, topic, author) = {
    let mut redis_scheduler = lock.lock();
    let existing_event = redis_scheduler.get_job(&job_id)?;

    let mut existing_event = match existing_event {
      Task::Event(item) => item,
      Task::Poll(_) => 
        return command_err!(format!("No event {} found", &job_id))
    };

    let author = existing_event.author;
    let id = existing_event.channel;

    let channel = match ChannelId(id).to_channel_cached(&ctx.cache) {
      Some(exists) => {
        match exists.guild() {
          Some(guild_channel) => guild_channel,
          None => return command_err!(format!("Channel {} not found", &id))
        }
      },
      None => return command_err!(format!("Channel {} not found", &id))
    };

    {
      let members = channel.read().members(&ctx.cache)?;

      if !members.iter().any(|m| m.user_id() == msg.author.id) {
        return command_err!(format!("No event {} found", &job_id))
      }
    };
    
    if !existing_event.members.contains(&msg.author.id.0) {
      let topic = existing_event.event.clone();

      existing_event.members.push(msg.author.id.0);
      redis_scheduler.edit_job(&Task::Event(existing_event), &job_id, None)?;
      (true, id, topic, author)
    } else {
      (false, 0, String::new(), author)
    }
  };

  if changed {
    ChannelId(channel_id).say(&ctx.http,
      format!("{} signed up for \"{}\" by {}", 
        msg.author.mention(), topic, UserId(author).mention()))?;
  }

  Ok(())
}
