#![macro_use]

use chrono::Duration;
use lazy_static::lazy_static;
use regex::Regex;
use serde_json::Value;
use serenity::model::prelude::*;

use serenity::{
  client::Context, model::interactions::application_command::ApplicationCommandInteraction,
};

#[macro_export]
macro_rules! error {
  ($type:expr, $value:expr) => {
    Err(Box::new(SerenityError::Url(format!(
      "Could not find {} {}",
      $type, $value
    ))))
  };
}

lazy_static! {
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

pub async fn get_guild(ctx: &Context, msg: &Message) -> Result<Guild, String> {
  match msg.guild(&ctx.cache) {
    Some(guild) => Ok(guild),
    None => Err(String::from("Could not find guild")),
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
      None => Err(String::from(fail_msg)),
    },
    None => Err(String::from(fail_msg)),
  }
}
