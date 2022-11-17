use crate::RedisConnectionKey;

use chrono::NaiveDate;
use redis::{AsyncCommands, RedisError};
use serenity::{
  builder::CreateApplicationCommands,
  model::application::{
    command::*,
    interaction::{application_command::*, *},
  },
  prelude::*,
};

use super::util::get_str_or_error;

pub fn birthday_command(
  commands: &mut CreateApplicationCommands,
) -> &mut CreateApplicationCommands {
  commands.create_application_command(|command| {
    command
      .name("birthday")
      .description("Set your birthday for getting a notification")
      .create_option(|op| {
        op.name("date")
          .kind(CommandOptionType::String)
          .description("Your birthday (in m/d/y), or None to remove")
          .required(true)
      })
  })
}

const DATE_OPTIONS: [&str; 8] = [
  "%m/%d/%y",   // 01/02/99
  "%m/%d/%Y",   // 01/02/1999,
  "%_m/%_d/%y", // 1/1/99
  "%_m/%_d/%Y", // 1/1/1999
  "%m/%_d/%y",  // 01/1/99
  "%m/%_d/%Y",  // 01/1/1999
  "%_m/%d/%y",  // 1/01/99
  "%_m/%d/%Y",  // 1/01/1999
];

pub const BIRTHDAY_KEY: &str = "birthdays";
pub const BIRTHDAY_FMT: &str = "%_m/%_d/%Y";

pub async fn interaction_birthday(
  ctx: &Context,
  interaction: &ApplicationCommandInteraction,
) -> Result<(), String> {
  let options = &interaction.data.options;

  if options.len() < 1 {
    return Err(String::from("Must provide a birthday"));
  } else if !interaction.guild_id.is_some() {
    return Err(String::from("Must be in a server"));
  }

  let user_id = interaction.user.id.0;

  let date_str = get_str_or_error(&options[0].value, "You must provide a birthday")?;

  if date_str == "None" {
    {
      let lock = {
        let mut context = ctx.data.write().await;
        context
          .get_mut::<RedisConnectionKey>()
          .expect("Expected redis connection")
          .clone()
      };

      let mut redis_client = lock.lock().await;
      let result: Result<(), RedisError> = redis_client.0.hdel(BIRTHDAY_KEY, user_id).await;

      if let Err(error) = result {
        return Err(error.to_string());
      }
    }

    let _ = interaction
      .create_interaction_response(ctx, |f| {
        f.kind(InteractionResponseType::ChannelMessageWithSource)
          .interaction_response_data(|msg| msg.content("Removed your birthday").ephemeral(true))
      })
      .await;
  } else {
    let mut result: Option<NaiveDate> = None;

    for option in DATE_OPTIONS {
      result = match NaiveDate::parse_from_str(&date_str, option) {
        Ok(result) => Some(result),
        Err(_) => None,
      };

      if result.is_some() {
        break;
      }
    }

    if result.is_none() {
      return Err(format!("Birthday '{}' is not a valid format", date_str));
    }

    let birthday = result.unwrap().format(BIRTHDAY_FMT).to_string();

    {
      let lock = {
        let mut context = ctx.data.write().await;
        context
          .get_mut::<RedisConnectionKey>()
          .expect("Expected redis connection")
          .clone()
      };

      let mut redis_client = lock.lock().await;
      let result: Result<(), RedisError> =
        redis_client.0.hset(BIRTHDAY_KEY, user_id, &birthday).await;

      if let Err(error) = result {
        return Err(error.to_string());
      }
    };

    let _ = interaction
      .create_interaction_response(ctx, |f| {
        f.kind(InteractionResponseType::ChannelMessageWithSource)
          .interaction_response_data(|msg| {
            msg
              .content(format!("Set your birthday to {}", birthday))
              .ephemeral(true)
          })
      })
      .await;
  }

  Ok(())
}
