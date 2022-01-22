#![macro_use]

use chrono::Duration;
use lazy_static::lazy_static;
use regex::Regex;
use serde_json::{Value};
use serenity::prelude::*;
use serenity::model::prelude::*;

use serenity::{
  client::Context,
  framework::standard::{CommandError, CommandResult},
  model::interactions::application_command::ApplicationCommandInteraction,
  utils::parse_mention,
};

#[inline]
pub async fn handle_command_err(ctx: &Context, msg: &Message, error: &str) -> CommandResult {
  if !msg.author.bot {
    let _ = msg.channel_id.say(&ctx.http, 
      &format!("Error in {:?}:\n{}", msg.content, error)).await;
  }

  Ok(())
}

#[macro_export]
macro_rules! error {
  ($type:expr, $value:expr) => {
    Err(Box::new(SerenityError::Url(format!("Could not find {} {}", $type, $value)))) 
  };
}

#[macro_export]
macro_rules! command_err {
  ($string:expr) => {
    Err(Box::new(SerenityError::Url(String::from($string))))
  };
}

lazy_static!{
  /// https://unicode.org/reports/tr51/#EBNF_and_Regex
  pub static ref EMOJI_REGEX: Regex = Regex::new(r"(?x)
    <a?:[a-zA-Z0-9_]+:[0-9]+>|
    \p{RI}\p{RI}|
    \p{Emoji} 
      ( \p{EMod} 
      | \x{FE0F} \x{20E3}? 
      | [\x{E0020}-\x{E007E}]+ \x{E007F} )?
      (\x{200D} \p{Emoji}
        ( \p{EMod} 
        | \x{FE0F} \x{20E3}? 
        | [\x{E0020}-\x{E007E}]+ \x{E007F} )?
      )*"
    ).unwrap();
}

#[inline]
pub fn get_user(interaction: &ApplicationCommandInteraction) -> &User {
  if let Some(member) = &interaction.member {
    &member.user
  } else {
    &interaction.user
  }
}

#[inline]
pub fn get_mention(interaction: &ApplicationCommandInteraction) -> String {
  if let Some(member) = &interaction.member {
    member.mention().to_string()
  } else {
    interaction.user.mention().to_string()
  }
}

pub fn format_duration(duration: &Duration) -> String {
  let mut duration = Duration::seconds(duration.num_seconds());
  let mut string = String::new();

  if duration.num_days() > 0 {
    string += &simple_pluralize("day", duration.num_days());
    duration = duration - Duration::days(duration.num_days());
  }

  if duration.num_hours() > 0 {
    if !string.is_empty() {
      string += ", ";
    }

    string += &simple_pluralize("hour", duration.num_hours());
    duration = duration - Duration::hours(duration.num_hours());
  }

  if duration.num_minutes() > 0 {
    if !string.is_empty() {
      string += ", ";
    }

    string += &simple_pluralize("minute", duration.num_minutes());
    duration = duration - Duration::minutes(duration.num_minutes());
  }

  if duration.num_seconds() > 0 {
    if !string.is_empty() {
      string += ", ";
    }

    string += &simple_pluralize("second", duration.num_seconds());
  }

  string
}

pub async fn get_channel_from_string(ctx: &Context, 
  guild: &Guild, name_or_id: &str) -> Result<ChannelId, CommandError> {

  match parse_mention(&name_or_id) {
    Some(id) => {
      let channel_id = ChannelId(id);
      match guild.channels.get(&channel_id) {
        Some(_) => Ok(channel_id),
        None => error!("channel", id)
      }
    },
    None => {
      match guild.channel_id_from_name(&ctx.cache, &name_or_id).await {
        Some(id) => Ok(id),
        None =>  error!("channel", name_or_id)
      }
    }
  }
}

pub async fn get_guild(ctx: &Context, msg: &Message) 
  -> Result<Guild, CommandError> {
  match msg.guild(&ctx.cache).await {
    Some(guild) => Ok(guild),
    None => command_err!("Could not find guild")
  }
}

pub fn get_role_from_string(guild: &Guild, name_or_id: &str) 
  -> Result<RoleId, CommandError> {
    
  match parse_mention(&name_or_id) {
    Some(id) => {
      let role_id = RoleId(id);
      match guild.roles.get(&role_id) {
        Some(_) => Ok(role_id),
        None => error!("role", id)
      }
    }
    None => {
      match guild.role_by_name(&name_or_id) {
        Some(role) => Ok(role.id),
        None => error!("role", &name_or_id)
      }
    }
  }
}

pub fn get_user_from_string(guild: &Guild,
  name_or_id: &str) -> Result<UserId, CommandError> {

  match parse_mention(&name_or_id) {
    Some(id) => { 
      let user_id = UserId(id);
      match guild.members.get(&user_id) {
        Some(_) => Ok(user_id),
        None => error!("user", id)
      }
    }
    None => {
      match guild.member_named(&name_or_id) {
        Some(member) => Ok(member.user.id),
        None => error!("user", name_or_id)
      }
    }
  }
}

#[inline]
fn simple_pluralize(msg: &str, count: i64) -> String {
  if count == 1 {
    format!("1 {}", msg)
  } else {
    format!("{} {}s", count, msg)
  }
}

#[inline]
pub fn get_str_or_error(op: &Option<Value>, fail_msg: &'static str) -> Result<String, String> {
  match op {
    Some(field) => match field.as_str() {
      Some(res) => Ok(String::from(res)),
      None => Err(String::from(fail_msg))
    },
    None => Err(String::from(fail_msg))
  }
}