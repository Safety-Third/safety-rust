use std::collections::HashSet;

use chrono::{Duration, Utc};
use chrono_tz::EST5EDT;
use lazy_static::lazy_static;
use regex::Regex;
use serenity::{
  builder::{CreateApplicationCommands, CreateComponents},
  model::{
    application::{command::*, interaction::application_command::*},
    prelude::{
      component::{ActionRowComponent, ButtonStyle, InputTextStyle},
      interaction::{
        message_component::MessageComponentInteraction, modal::ModalSubmitInteraction,
        InteractionResponseType,
      },
      ChannelId, Message, ReactionType,
    },
  },
  prelude::*,
  utils::{Color, Colour},
};

use super::util::{format_duration, get_str_or_error, get_user};
use crate::util::scheduler::{Callable, Poll, RedisSchedulerKey, EMOJI_ORDER, MAX_OPTIONS};

pub fn poll_command(commands: &mut CreateApplicationCommands) -> &mut CreateApplicationCommands {
  commands.create_application_command(|command|
    command.name("poll")
      .description("Creates an emoji-based poll for a certain topic.")
      .create_option(|new| {
        let mut new = new.name("new")
          .kind(CommandOptionType::SubCommand)
          .description("Create a new poll")
          .create_sub_option(|topic| topic
            .name("topic")
            .kind(CommandOptionType::String)
            .description("The topic of this poll")
            .required(true)
          )
          .create_sub_option(|time| time
            .name("time")
            .kind(CommandOptionType::String)
            .description("Time in the form 'X days, X hours, X minutes'. At least one of days, hours, or minutes required.")
            .required(true)
          )
          .create_sub_option(|allow_others| allow_others
            .name("allow_others_to_add_options")
            .kind(CommandOptionType::Boolean)
            .description("Whether to allow other users in the same channel to add options to this poll")
            .required(true)
          )
          .create_sub_option(|allow_others| allow_others
            .name("pin")
            .kind(CommandOptionType::Boolean)
            .description("Whether to pin this poll")
            .required(true)
          );

        for idx in 1..=MAX_OPTIONS {
          new = new.create_sub_option(|op| op
            .name(format!("option-{}", idx))
            .kind(CommandOptionType::String)
            .description(format!("poll option {}", idx))
            .required(idx < 3)
          );
        }

        new
      })
      .create_option(|add| {
        let mut add = add.name("options_add")
          .kind(CommandOptionType::SubCommand)
          .description("Add one or more options to an existing poll")
          .create_sub_option(|id| id
            .name("poll_id")
            .kind(CommandOptionType::String)
            .description("The id of the poll you wish to edit")
            .required(true)
          );

        for idx in 1..=18 {
          add = add.create_sub_option(|op| op
            .name(format!("option-{}", idx))
            .kind(CommandOptionType::String)
            .description(format!("poll option {}", idx))
            .required(idx == 1)
          );
        }

        add
      })
  )
}

const BOOL_FAIL_MESSAGE: &str =
  "You must say whether others are allowed to add to this poll or not";

pub async fn interaction_poll(
  ctx: &Context,
  interaction: &ApplicationCommandInteraction,
) -> Result<(), String> {
  let data = &interaction.data;

  if data.options.len() < 1 {
    return Err(String::from("Must have subcommand"));
  }

  match interaction.data.options[0].name.as_str() {
    "new" => new_poll(ctx, interaction).await,
    "options_add" => option_add(ctx, interaction).await,
    _ => Err(String::from("Unexpected command")),
  }
}

const FORMAT_STRINGS: [&str; 20] = [
  "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "A", "B", "C", "D", "E", "F", "G", "H", "I",
  "J",
];

async fn new_poll(
  ctx: &Context,
  interaction: &ApplicationCommandInteraction,
) -> Result<(), String> {
  let data_options = &interaction.data.options[0].options;

  if data_options.len() < 4 {
    return Err(String::from(
      "You must have a topic, date, and at least two options",
    ));
  }

  let user = get_user(&interaction);
  let mention = user.mention().to_string();
  let author_id = user.id.0;

  let channel = interaction.channel_id;

  let duration = {
    let date_str = get_str_or_error(&data_options[1].value, "You must provide a time")?;
    parse_time(&date_str.trim())?
  };

  let topic_str = get_str_or_error(&data_options[0].value, "You must provide a topic")?;
  let mut options: Vec<String> = vec![];
  let mut existing: HashSet<String> = HashSet::new();

  let allow_others = match &data_options[2].value {
    Some(field) => match field.as_bool() {
      Some(boolean) => boolean,
      None => return Err(String::from(BOOL_FAIL_MESSAGE)),
    },
    None => return Err(String::from(BOOL_FAIL_MESSAGE)),
  };

  let pin = match &data_options[3].value {
    Some(field) => match field.as_bool() {
      Some(boolean) => boolean,
      None => return Err(String::from(BOOL_FAIL_MESSAGE)),
    },
    None => return Err(String::from(BOOL_FAIL_MESSAGE)),
  };

  for option in &data_options[4..] {
    if let Some(op) = &option.value {
      if let Some(op_str) = op.as_str() {
        let new_option = String::from(op_str.trim());

        if !existing.contains(&new_option) {
          existing.insert(new_option.clone());
          options.push(new_option);
        }

        continue;
      }
    }

    return Err(format!(
      "Error parsing field {}. The value was {:?}",
      option.name, option.value
    ));
  }
  if options.len() > EMOJI_ORDER.len() {
    return Err(format!("You cannot have more than {} emojis", MAX_OPTIONS));
  }

  let time = (Utc::now() + duration).with_timezone(&EST5EDT);

  let reactions: Vec<ReactionType> = EMOJI_ORDER[0..options.len()]
    .iter()
    .map(|emoji| ReactionType::Unicode(emoji.to_string()))
    .collect();

  let lock = {
    let mut context = ctx.data.write().await;
    context
      .get_mut::<RedisSchedulerKey>()
      .expect("Expected redis scheduler")
      .clone()
  };

  let poll_id = {
    let mut redis_scheduler = lock.lock().await;
    match redis_scheduler.reserve_id().await {
      Ok(id) => id,
      Err(error) => return Err(error.to_string()),
    }
  };

  if let Err(error) = interaction
    .create_interaction_response(ctx, |resp| {
      resp
        .kind(InteractionResponseType::ChannelMessageWithSource)
        .interaction_response_data(|msg| {
          msg
            .content(format!(
              "{} Poll: '{}'\n**BEGIN DISCUSSION**",
              &mention, &topic_str
            ))
            .embed(|e| {
              let mut description = String::from(">>> ");

              for (count, option) in options.iter().enumerate() {
                description += &format!("{}. {}\n", FORMAT_STRINGS[count], &option);
              }

              let time_str = time.format("%D %r %Z");

              e.color(Color::BLITZ_BLUE)
                .title(format!("Poll: {}", &topic_str))
                .field("duration", format_duration(&duration), true)
                .field("poll id", &poll_id, true)
                .field("ends at", time_str, false)
                .field(
                  "Others can edit",
                  match allow_others {
                    true => "Yes",
                    false => "No",
                  },
                  false,
                )
                .description(description)
            })
            .components(components(allow_others))
        })
    })
    .await
  {
    return Err(error.to_string());
  }

  let message = match interaction.get_interaction_response(ctx).await {
    Ok(msg) => msg,
    Err(error) => return Err(error.to_string()),
  };

  if pin {
    let _ = message.pin(&ctx).await;
  }

  for react in reactions {
    let _ = message.react(ctx, react).await;
  }

  let poll = Poll {
    author: author_id,
    channel: channel.0,
    message: message.id.0,
    others: allow_others,
    topic: topic_str,
  };

  {
    let mut redis_scheduler = lock.lock().await;
    match redis_scheduler
      .schedule_job(&poll, &poll_id, time.timestamp(), duration.num_seconds())
      .await
    {
      Ok(_) => Ok(()),
      Err(error) => Err(error.to_string()),
    }
  }
}

enum Inter<'t> {
  App(&'t ApplicationCommandInteraction),
  Modal(&'t ModalSubmitInteraction),
}

async fn option_add(
  ctx: &Context,
  interaction: &ApplicationCommandInteraction,
) -> Result<(), String> {
  let data_options = &interaction.data.options[0].options;

  if data_options.len() < 2 {
    return Err(String::from(
      "You must have a poll ID and at least one new option",
    ));
  }

  let poll_id = get_str_or_error(&data_options[0].value, "You must provide a poll ID")?;

  let mut options: Vec<String> = vec![];

  for option in &data_options[1..] {
    if let Some(op) = &option.value {
      if let Some(op_str) = op.as_str() {
        let trimmed = op_str.trim();

        if trimmed != "" {
          options.push(String::from(op_str.trim()));
        }

        continue;
      }
    }

    return Err(format!(
      "Error parsing field {}. The value was {:?}",
      option.name, option.value
    ));
  }

  do_option_add(
    ctx,
    Inter::App(interaction),
    interaction.channel_id.0,
    interaction.user.id.0,
    poll_id,
    options,
  )
  .await
}

async fn do_option_add<'t>(
  ctx: &Context,
  interaction: Inter<'t>,
  channel_id: u64,
  user_id: u64,
  poll_id: String,
  options: Vec<String>,
) -> Result<(), String> {
  if options.len() == 0 {
    return Err(String::from("You must provide at least one option"));
  }

  let poll: Poll = {
    let lock = {
      let mut context = ctx.data.write().await;
      context
        .get_mut::<RedisSchedulerKey>()
        .expect("Expected redis scheduler")
        .clone()
    };

    let mut redis_scheduler = lock.lock().await;
    match redis_scheduler.get_job(&poll_id).await {
      Ok(result) => result,
      Err(error) => return Err(error.to_string()),
    }
  };

  if poll.others {
    if poll.channel != channel_id {
      return Err(String::from(
        "You must be in the same channel to add an option to a poll",
      ));
    }
  } else if poll.author != user_id {
    return Err(String::from("Only the creator of a poll can edit it"));
  }

  let mut message: Message = match ChannelId(poll.channel)
    .message(&ctx.http, poll.message)
    .await
  {
    Ok(msg) => msg,
    Err(error) => {
      return Err(format!(
        "An error occurred when trying to fetch the poll: {}",
        error
      ))
    }
  };

  if message.embeds.is_empty() {
    return Err(String::from(
      "Message does not have embeds; something has gone terribly wrong",
    ));
  }

  let mut existing_options: Vec<&str> = vec![];
  let mut all_options: HashSet<&str> = HashSet::new();
  let embed = &message.embeds[0];

  if let Some(content) = &embed.description {
    for option in content.split('\n') {
      let index = match option.find('.') {
        Some(loc) => loc + 2,
        None => 0,
      };

      let option_string = &option[index..];

      all_options.insert(option_string);
      existing_options.push(option_string);
    }
  }

  let mut description = embed.description.clone().unwrap_or(String::from(""));
  let mut added: Vec<&str> = vec![];

  let existing_len = existing_options.len();
  let mut count = 0;

  {
    let offset = 1 + existing_len;

    for option in &options {
      let op_as_string = &option.as_str();
      if !all_options.contains(op_as_string) {
        added.push(option);
        description += &format!("\n{}. {}", FORMAT_STRINGS[count + offset], &option);
        all_options.insert(op_as_string);
        count += 1;
      }
    }
  }

  if all_options.len() > MAX_OPTIONS {
    let smart_plural = if added.len() == 1 {
      "option"
    } else {
      "options"
    };
    return Err(format!(
      "There are already {} options. Adding an additional {} {} will exceed the 20 option limit.",
      existing_len,
      added.len(),
      smart_plural
    ));
  }

  if count == 0 {
    let _ = match interaction {
      Inter::App(data) => {
        data
          .create_interaction_response(ctx, |resp| {
            resp
              .kind(InteractionResponseType::ChannelMessageWithSource)
              .interaction_response_data(|msg| {
                msg
                  .content(
                    "All the options you asked to add already exist; no changes have been made.",
                  )
                  .ephemeral(true)
              })
          })
          .await
      }
      Inter::Modal(data) => {
        data
          .create_interaction_response(ctx, |resp| {
            resp
              .kind(InteractionResponseType::ChannelMessageWithSource)
              .interaction_response_data(|msg| {
                msg
                  .content(
                    "All the options you asked to add already exist; no changes have been made.",
                  )
                  .ephemeral(true)
              })
          })
          .await
      }
    };

    return Ok(());
  }

  let ending_len = existing_len + count;

  let title = embed.title.clone().unwrap_or(String::from(""));
  let fields: Vec<(String, String, bool)> = embed
    .fields
    .iter()
    .map(|field| (field.name.clone(), field.value.clone(), field.inline))
    .collect();

  match message
    .edit(&ctx.http, |msg| {
      msg.embed(|e| {
        e.color(Color::BLITZ_BLUE)
          .title(title)
          .fields(fields)
          .description(description)
      })
    })
    .await
  {
    Ok(_) => {
      let _ = match interaction {
        Inter::App(data) => {
          data
            .create_interaction_response(ctx, |resp| {
              resp
                .kind(InteractionResponseType::ChannelMessageWithSource)
                .interaction_response_data(|msg| {
                  msg.content(format!(
                    "New options for poll **{}** by <@{}>:\n>>> {}",
                    poll_id,
                    user_id,
                    added.join("\n")
                  ))
                })
            })
            .await
        }
        Inter::Modal(data) => {
          data
            .create_interaction_response(ctx, |resp| {
              resp
                .kind(InteractionResponseType::ChannelMessageWithSource)
                .interaction_response_data(|msg| {
                  msg.content(format!(
                    "New options for poll **{}** by <@{}>:\n>>> {}",
                    poll_id,
                    user_id,
                    added.join("\n")
                  ))
                })
            })
            .await
        }
      };

      let reactions: Vec<ReactionType> = EMOJI_ORDER[existing_len..ending_len]
        .iter()
        .map(|emoji| ReactionType::Unicode(emoji.to_string()))
        .collect();

      for react in reactions {
        let _ = message.react(ctx, react).await;
      }

      Ok(())
    }
    Err(error) => Err(format!("Could not edit poll: {}", error)),
  }
}

pub async fn handle_poll_interaction(
  ctx: &Context,
  interaction: &MessageComponentInteraction,
) -> Result<(), String> {
  let msg = &interaction.message;

  // are you the author?
  if msg.mentions.len() == 1 && interaction.user == msg.mentions[0] {
    match interaction.data.custom_id.as_str() {
      "close" => {
        let poll = {
          let lock = {
            let mut context = ctx.data.write().await;
            context
              .get_mut::<RedisSchedulerKey>()
              .expect("Expected redis instance")
              .clone()
          };

          let mut redis_scheduler = lock.lock().await;
          redis_scheduler
            .pop_job(&msg.embeds[0].fields[1].value)
            .await
        };

        match poll {
          Ok(result) => {
            let _ = interaction
              .create_interaction_response(ctx, |resp| {
                resp
                  .kind(InteractionResponseType::UpdateMessage)
                  .interaction_response_data(|resp| resp.components(|comp| comp))
              })
              .await;

            if let Some(real_poll) = result {
              real_poll.call(&ctx.http).await;
            }

            Ok(())
          }
          Err(error) => Err(format!(
            "An error occurred when trying to close the poll: {}",
            error
          )),
        }
      }
      "delete" => {
        if let Err(error) = msg.delete(ctx).await {
          Err(format!("Could not delete poll: {}", error))
        } else {
          let lock = {
            let mut context = ctx.data.write().await;
            context
              .get_mut::<RedisSchedulerKey>()
              .expect("Expected redis instance")
              .clone()
          };

          {
            let mut redis_scheduler = lock.lock().await;
            let _ = redis_scheduler
              .remove_job(&msg.embeds[0].fields[1].value)
              .await;
          };

          Ok(())
        }
      }
      _ => Ok(()),
    }
  } else {
    nop(ctx, interaction).await;
    Ok(())
  }
}

pub async fn handle_poll_options_toggle(
  ctx: &Context,
  interaction: &MessageComponentInteraction,
) -> Result<(), String> {
  let msg = &interaction.message;
  let msg_id = &msg.embeds[0].fields[1].value;

  if interaction.user != msg.mentions[0] {
    nop(ctx, interaction).await;
    return Ok(());
  }

  let allowing_others = {
    let lock = {
      let mut context = ctx.data.write().await;
      context
        .get_mut::<RedisSchedulerKey>()
        .expect("Expected redis instance")
        .clone()
    };

    let mut redis_scheduler = lock.lock().await;
    redis_scheduler
      .edit_job(msg_id, |mut f| {
        let will_allow_others = !f.others;
        f.others = will_allow_others;
        (f, will_allow_others)
      })
      .await
      .map_err(|e| e.to_string())?
  };

  let embed = &interaction.message.embeds[0];

  let mut fields: Vec<(String, String, bool)> = embed
    .fields
    .iter()
    .map(|field| (field.name.clone(), field.value.clone(), field.inline))
    .collect();

  let poll_id = fields[1].1.clone();

  if 3 < fields.len() {
    fields[3].1 = String::from(match allowing_others {
      true => "Yes",
      false => "No",
    });
  } else {
    return Err(String::from(
      "Not enough fields provided; something has gone horribly wrong",
    ));
  }

  let _ = interaction
    .message
    .clone()
    .edit(&ctx, |m| {
      m.embed(|e| {
        e.color(Colour::BLITZ_BLUE)
          .title(embed.title.clone().unwrap_or(String::from("")))
          .description(embed.description.clone().unwrap_or(String::from("")))
          .fields(fields)
      })
      .components(components(allowing_others))
    })
    .await;

  let _ = interaction
    .create_interaction_response(ctx, |resp| {
      resp
        .kind(InteractionResponseType::ChannelMessageWithSource)
        .interaction_response_data(|msg| {
          msg.content(format!(
            "{} has set poll {} to editable by **{}**",
            interaction.user.mention(),
            poll_id,
            match allowing_others {
              true => "everyone",
              false => "author only",
            }
          ))
        })
    })
    .await;

  Ok(())
}

pub async fn handle_poll_add(
  ctx: &Context,
  interaction: &MessageComponentInteraction,
) -> Result<(), String> {
  let msg = &interaction.message;
  let msg_id = &msg.embeds[0].fields[1].value;

  let poll = match {
    let lock = {
      let mut context = ctx.data.write().await;
      context
        .get_mut::<RedisSchedulerKey>()
        .expect("Expected redis instance")
        .clone()
    };

    let mut redis_scheduler = lock.lock().await;
    redis_scheduler.get_job(msg_id).await
  } {
    Ok(poll) => poll,
    Err(error) => {
      return Err(format!(
        "An error occurred when trying to close the poll: {}",
        error
      ))
    }
  };

  if poll.others || msg.mentions.len() == 1 && interaction.user == msg.mentions[0] {
    let _ = interaction
      .create_interaction_response(ctx, |response| {
        response
          .kind(InteractionResponseType::Modal)
          .interaction_response_data(|msg| {
            msg
              .ephemeral(true)
              .title("Add one or more options")
              .custom_id("options_add")
              .components(|comp| {
                comp.create_action_row(|row| {
                  row.create_input_text(|text| {
                    text
                      .custom_id(msg_id)
                      .label("Your options")
                      .placeholder("Please have one option per line")
                      .required(true)
                      .style(InputTextStyle::Paragraph)
                  })
                })
              })
          })
      })
      .await;

    Ok(())
  } else {
    nop(ctx, interaction).await;
    Ok(())
  }
}

pub async fn interaction_poll_add_followup(
  ctx: &Context,
  modal: &ModalSubmitInteraction,
) -> Result<(), String> {
  let (id, ops) =
    if let ActionRowComponent::InputText(text) = &modal.data.components[0].components[0] {
      (text.custom_id.clone(), text.value.clone())
    } else {
      return Err(String::from("You must provide some options"));
    };

  let options: Vec<String> = ops
    .split("\n")
    .filter_map(|f| {
      let text = f.trim();

      if text != "" {
        Some(text.to_owned())
      } else {
        None
      }
    })
    .collect();

  do_option_add(
    ctx,
    Inter::Modal(modal),
    modal.channel_id.0,
    modal.user.id.0,
    id,
    options,
  )
  .await
}

const TIMING_ERROR_STR: &str = "It looks like you provided the wrong time string.
The accepted format is: `X days, X hours, X minutes`, where `X` is a non-negative number.
You can provide any of the three times (e.g., `2 days, 1 minute`, `40 hours`), but must give **at least one**.
Please note that **order does matter** (first days, then hours, then minutes).
You can also write this string in shorthand! All spacing and commas are optional.
In addition, there are shortcuts for each of the times, day, hour, and minute:
> - `day`: d, ds, day, days
> - `hour`: h, hr, hrs, hour, hours
> - `minute`: m, min, mins, minute, minutes";

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
    static ref MIN_DURATION: Duration = Duration::seconds(60);
    static ref CHAR_REGEX: Regex = Regex::new(r"[a-zA-Z\s,]").unwrap();
    static ref RE: Regex = Regex::new(
      r"(?x)
      # Match any of the following: 1d, 2ds, 1 day, 2 days (spacing optional)
      (?P<days>\d+ \s? +d (ay s?)?)? ,? \s*
      # Match any of the following: 1h, 1 hr, 2 hrs, 1hour, 4 hours
      (?P<hours>\d+ \s? (hour s? | hr s?| h))? ,? \s*
      # Match any of the following: 1 m, 2 mins, 1 minute, 40minutes
      (?P<minutes>\d+ \s? m (in (ute)? s?)?)?$"
    )
    .unwrap();
  }

  if let Ok(time_in_minutes) = timing.parse::<i64>() {
    return Ok(Duration::minutes(time_in_minutes));
  }

  let caps = match RE.captures(timing) {
    Some(captures) => captures,
    None => return Err(TIMING_ERROR_STR),
  };

  let mut duration = Duration::zero();
  let mut passed = false;

  if let Some(days) = caps.name("days") {
    let days_str = CHAR_REGEX.replace_all(&days.as_str(), "");

    match days_str.parse::<i64>() {
      Ok(days_int) => {
        duration = duration + Duration::days(days_int);
        passed = true;
      }
      Err(_) => return Err("Must provide a numeric value for days"),
    }
  }

  if let Some(hours) = caps.name("hours") {
    let hours_str = CHAR_REGEX.replace_all(&hours.as_str(), "");

    match hours_str.parse::<i64>() {
      Ok(hours_int) => {
        duration = duration + Duration::hours(hours_int);
        passed = true;
      }
      Err(_) => return Err("Must provide a numeric value for hours"),
    }
  }

  if let Some(minutes) = caps.name("minutes") {
    let minutes_str = CHAR_REGEX.replace_all(&minutes.as_str(), "");

    match minutes_str.parse::<i64>() {
      Ok(minutes_int) => {
        duration = duration + Duration::minutes(minutes_int);
        passed = true;
      }
      Err(_) => return Err("Must provide a numeric value for minutes"),
    }
  }

  if !passed {
    Err(TIMING_ERROR_STR)
  } else if duration < *MIN_DURATION {
    Err("Poll must be at least 1 minute")
  } else {
    Ok(duration)
  }
}

async fn nop(ctx: &Context, interaction: &MessageComponentInteraction) {
  let _ = interaction
    .create_interaction_response(ctx, |resp| {
      resp.kind(InteractionResponseType::DeferredUpdateMessage)
    })
    .await;
}

fn components(allow_others: bool) -> impl Fn(&mut CreateComponents) -> &mut CreateComponents {
  move |comp| {
    comp.create_action_row(|row| {
      row
        .create_button(|button| {
          button
            .style(ButtonStyle::Danger)
            .label("Delete this poll")
            .custom_id("delete")
        })
        .create_button(|button| {
          button
            .style(ButtonStyle::Secondary)
            .label("Close this poll")
            .custom_id("close")
        })
        .create_button(|button| {
          button
            .style(ButtonStyle::Primary)
            .label("Add an option")
            .custom_id("add")
        })
        .create_button(|button| {
          button
            .style(ButtonStyle::Secondary)
            .label(match allow_others {
              true => "Only you add options",
              false => "Let all add options",
            })
            .custom_id("toggle")
        })
    })
  }
}
