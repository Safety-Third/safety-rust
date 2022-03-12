#![macro_use]
use std::collections::{BTreeMap, HashMap};

use redis::AsyncCommands;
use serde_json::{Value, json};
use serenity::{
  model::prelude::interactions::application_command::*, model::prelude::*,
  prelude::*
};

use crate::util::scheduler::{RedisConnectionKey};

pub fn stats_commands() -> Value {
  let guild_id = json!({
    "type": ApplicationCommandOptionType::String,
    "name": "server",
    "description": "The server to access (optional). Defaults to the current server otherwise",
    "required": "false"
  });

  return json!({
    "name": "stats",
    "description": "View and manage reaction stats",
    "options": [json!({
      "name": "consent",
      "type": ApplicationCommandOptionType::SubCommandGroup,
      "description": "Manage consent for collecting reaction stats",
      "options": [json!({
        "name": "approve",
        "type": ApplicationCommandOptionType::SubCommand,
        "description": "Approve of collecting stats of reactions in a server",
        "options": [guild_id]
      }), json!({
        "name": "delete",
        "type": ApplicationCommandOptionType::SubCommand,
        "description": "Revoke permission and immediately delete all collected reaction stats in a server",
        "options": [guild_id]
      }), json!({
        "name": "revoke",
        "type": ApplicationCommandOptionType::SubCommand,
        "description": "Revoke collecting reaction stats in a given server",
        "options": [guild_id]
      })]
    }), json!({
      "name": "get",
      "type": ApplicationCommandOptionType::SubCommand,
      "description": "See how often you use one or more emojis in a server. These stats are direct messaged",
      "options": [guild_id, json!({
        "name": "number_of_emojis",
        "type": ApplicationCommandOptionType::Integer,
        "description": "The number of emojis to show (default top 10)",
        "required": false
      }), json!({
        "name": "emoji",
        "type": ApplicationCommandOptionType::String,
        "description": "An emoji to query",
        "required": false
      })]
    })]
  })
}

pub async fn interaction_stats_entrypoint(ctx: &Context,
  interaction: &ApplicationCommandInteraction) -> Result<(), String> {

  let data = &interaction.data;

  if data.options.len() < 1 {
    return Err(String::from("You must provide a subcommand group"))
  }


  match interaction.data.options[0].name.as_str() {
    "consent" => intereaction_stats_consent(ctx, interaction).await,
    "get" => intereaction_stats_usage(ctx, interaction).await,
    _ => Err(String::from("Unexpected command"))
  }
}

enum StatsCommand {
  Approve, Delete, Revoke
}

async fn intereaction_stats_consent(ctx: &Context,
  interaction: &ApplicationCommandInteraction) -> Result<(), String> {

  let subdata = &interaction.data.options[0].options;

  if subdata.len() < 1 {
    return Err(String::from("You must provide a subcommand"))
  }
  
  let subcommand = match subdata[0].name.as_str() {
    "approve" => StatsCommand::Approve,
    "delete" => StatsCommand::Delete,
    "revoke" => StatsCommand::Revoke,
    _ => return Err(String::from("Invalid subcommand"))
  };

  let subsubdata = &subdata[0].options;

  let id_or_name_key = if subsubdata.len() > 0 {
    match subsubdata[0].value {
      Some(ref name) => name.as_str(),
      None => None
    }
  } else {
    None
  };

  let (guild_name, redis_key) = get_name_and_key_from_interaction(ctx, interaction, id_or_name_key).await?;

  let lock = {
    let mut context = ctx.data.write().await;
    context.get_mut::<RedisConnectionKey>()
      .expect("Expected redis connection")
      .clone()
  };

  let result = {
    let mut redis_client = lock.lock().await;

    match subcommand {
      StatsCommand::Approve => {
        match redis_client.0.hset::<&str, &str, u64, u64>(&redis_key, "consent", 1).await {
          Ok(_) => format!("You have consented to record stats of your reactions for **{}**", guild_name),
          Err(error) => return Err(error.to_string())
        }
      },
      StatsCommand::Delete => {
        match redis_client.0.del::<&str, u64>(&redis_key).await {
          Ok(_) => format!("You have revoked consent of recording your reaction usage and deleted all saved data for **{}**", guild_name),
          Err(error) => return Err(error.to_string())
        }
      },
      StatsCommand::Revoke => {
        match redis_client.0.hset::<&str, &str, u64, u64>(&redis_key, "consent", 0).await {
          Ok(_) => format!("You have revoked consent of recording your reaction usage for **{}**. If you wish to also delete your stats, you may use /stats consent delete", guild_name),
          Err(error) => return Err(error.to_string())
        }
      },
    }
  };

  if let Err(why) = interaction.user.direct_message(ctx, |msg| msg.content(result)).await {
    eprintln!("Error sending DM: {:?}", why);
    Err(String::from("There was an error DMing you"))
  } else {
    match interaction.create_interaction_response(ctx, |resp| {
      resp.kind(InteractionResponseType::ChannelMessageWithSource)
        .interaction_response_data(|msg| msg.content("OK"))
    }).await {
      Err(error) => Err(error.to_string()),
      Ok(_) => Ok(())
    }
  }
}

async fn intereaction_stats_usage(ctx: &Context,
  interaction: &ApplicationCommandInteraction) -> Result<(), String> {

  let subdata = &interaction.data.options[0].options;

  let mut emoji: Option<&str> = None;
  let mut emoji_count: Option<i64> = None;
  let mut id_or_name: Option<&str> = None;

  for option in subdata {
    if let Some(ref value) = option.value {
      match option.name.as_str() {
        "emoji" => emoji = value.as_str(),
        "number_of_emojis" => emoji_count = value.as_i64(),
        "server" => id_or_name = value.as_str(),
        _ => {}
      };
    }
  }

  if let Some(ref count) = emoji_count {
    if count < &1 {
      return Err(String::from("The number of emojis should at least be one"))
    }
  }

  let (guild_name, redis_key) = get_name_and_key_from_interaction(ctx, interaction, id_or_name).await?;

  let lock = {
    let mut context = ctx.data.write().await;
    context.get_mut::<RedisConnectionKey>()
      .expect("Expected redis connection")
      .clone()
  };

  let react_stats: HashMap<String, u64> = {
    let mut conn = lock.lock().await;
    match conn.0.hgetall(redis_key).await {
      Ok(res) => res,
      Err(error) => return Err(error.to_string())
    }
  };

  let emoji_limit = emoji_count.unwrap_or(5);

  let result = if react_stats.len() <= 1 {
    String::from("You have used no emojis")
  } else {
    if let Some(specific_emoji) = emoji {
      let react_count = react_stats.get(specific_emoji).unwrap_or(&0);

      if react_count == &1 {
        format!("You have used {} 1 time in {}", specific_emoji, guild_name)
      } else {
        format!("You have used {} {} times in {}", specific_emoji, react_count, guild_name)
      }
    } else {
      let mut message = format!("Stats in {}\n>>> ", guild_name);
      let mut score_mapping: BTreeMap<&u64, Vec<&str>> = BTreeMap::new();
  
      for (emoji, count) in react_stats.iter() {
        if emoji == "consent" {
          continue
        }
  
        match score_mapping.get_mut(count) {
          Some(list) => list.push(emoji),
          None => {
            score_mapping.insert(count, vec![emoji]);
          }
        };
      }

      let mut emoji_count = 0;
  
      for (count, emojis) in score_mapping.iter().rev() {
        let emoji_len = emojis.len() as i64;
  
        if emoji_len + emoji_count >= emoji_limit * 2 {
          break;
        }
        
        message += &format!("{}: {}\n", count, &emojis.join(" "));
        emoji_count += emoji_len;
  
        if emoji_count >= emoji_limit {
          break;
        }
      }

      message
    }
  };

  if let Err(why) = interaction.user.direct_message(ctx, |msg| msg.content(result)).await {
    eprintln!("Error sending DM: {:?}", why);
    Err(String::from("There was an error DMing you"))
  } else {
    match interaction.create_interaction_response(ctx, |resp| {
      resp.kind(InteractionResponseType::ChannelMessageWithSource)
        .interaction_response_data(|msg| msg.content("OK"))
    }).await {
      Err(error) => Err(error.to_string()),
      Ok(_) => Ok(())
    }
  }
}

async fn get_name_and_key_from_interaction(ctx: &Context, interaction: &ApplicationCommandInteraction, 
  id_or_name: Option<&str>) -> Result<(String, String), String> {
  match id_or_name {
    Some(identifier) => {
      match get_guild_from_id(ctx, &identifier).await {
        Some(guild) => {
          Ok((guild.name.clone(), format!("{}:{}", interaction.user.id.0, guild.id.0)))
        },
        None => Err(format!("Could not find a server {}", identifier))
      }
    },
    None => {
      if let Some(id) = interaction.guild_id {
        if let Some(name) = id.name(&ctx.cache).await {
          Ok((name, format!("{}:{}", interaction.user.id.0, id)))
        } else {
          Err(String::from("Could not get guild name"))
        }
      } else {
        Err(String::from("You must provide a guild id/name. Alternatively, you can message in a server )channel"))
      }
    }
  }
}

async fn get_guild_from_id(ctx: &Context, id_or_name: &str) -> Option<Guild> {
  match id_or_name.parse::<u64>() {
    Ok(id) => ctx.cache.guild(id).await,
    Err(_) => {
      let guilds = &ctx.cache.guilds().await;

      for guild in guilds.iter() {
        if let Some(name) = guild.name(&ctx.cache).await {
          if name == id_or_name {
            return guild.to_guild_cached(&ctx.cache).await;
          }
        }
      }

      None
    }
  }
}
