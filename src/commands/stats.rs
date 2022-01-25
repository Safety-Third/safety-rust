#![macro_use]
use std::collections::{BTreeMap, HashMap};

use redis::AsyncCommands;
use serde_json::{Value, json};
use serenity::{
  framework::standard::{Args, CommandError, CommandResult, macros::command},
  model::prelude::interactions::application_command::*, model::prelude::*,
  prelude::*
};

use crate::util::scheduler::{RedisConnectionKey};
use super::util::{get_guild, handle_command_err};

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

  if let Err(why) = interaction.user.direct_message(&ctx.http, |msg| msg.content(result)).await {
    eprintln!("Error sending DM: {:?}", why);
    Err(String::from("There was an error DMing you"))
  } else {
    match interaction.create_interaction_response(&ctx.http, |resp| {
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

  if let Err(why) = interaction.user.direct_message(&ctx.http, |msg| msg.content(result)).await {
    eprintln!("Error sending DM: {:?}", why);
    Err(String::from("There was an error DMing you"))
  } else {
    match interaction.create_interaction_response(&ctx.http, |resp| {
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


const DEFAULT_EMOJI_LIMIT: u64 = 5;

#[command]
#[max_args(1)]
#[usage("optional_server_name_id")]
#[example("000000000000000000")]
#[example("\"test server\"")]
#[example("")]
/// Consent to have this bot record stats of your emoji usage. These start are NOT anonymous
/// 
/// This can be called in a server to consent to recording stats in that server, 
/// or you can provide a server ID/name to consent via a DM with this bot.
pub async fn consent(ctx: &Context, msg: &Message, mut args: Args) -> CommandResult {
  let id_or_name = if !args.is_empty() {
    Some(args.single_quoted::<String>()?)
  } else {
    None
  };

  let (guild_name, key) = get_name_and_key(ctx, msg, id_or_name).await?;

  let lock = {
    let mut context = ctx.data.write().await;
    context.get_mut::<RedisConnectionKey>()
      .expect("Expected redis connection")
      .clone()
  };

  if let Err(error) = {
    let mut redis_clinet = lock.lock().await;
    redis_clinet.0.hset::<&str, &str, u64, u64>(&key, "consent", 1).await
  } {
    return handle_command_err(ctx, msg, &error.to_string()).await;
  };

  let _ = msg.author.dm(&ctx.http, |m| {
    m.content(
      &format!("You have consented to record stats of your reactions\nin {}", guild_name))
  }).await;

  Ok(())
}

#[command]
#[max_args(1)]
#[usage("optional_server_name_id")]
#[example("000000000000000000")]
#[example("\"test server\"")]
#[example("")]
/// Delete all emoji stats associated with a certain server.
///
/// This can be called in a server to deleta all stats in that server, 
/// or you can provide a server ID/name to delete all stats via a DM with this bot.
pub async fn delete(ctx: &Context, msg: &Message, mut args: Args) -> CommandResult {
  let id_or_name = if !args.is_empty() {
    Some(args.single_quoted::<String>()?)
  } else {
    None
  };

  let (guild_name, key) = get_name_and_key(ctx, msg, id_or_name).await?;

  let lock = {
    let mut context = ctx.data.write().await;
    context.get_mut::<RedisConnectionKey>()
      .expect("Expected redis connection")
      .clone()
  };

  if let Err(error) = { 
    let mut redis_clinet = lock.lock().await;
    redis_clinet.0.del::<&str, u64>(&key).await
  } {
    return handle_command_err(ctx, msg, &error.to_string()).await
  }

  let _ = msg.author.dm(&ctx.http, |m| {
    m.content(
      &format!("You deleted all of your stats\nin {}", guild_name))
  }).await;

  Ok(())
}

#[command]
#[max_args(1)]
#[usage("optional_server_name_id")]
#[example("000000000000000000")]
#[example("\"test server\"")]
#[example("")]
/// Revoke your consent for continued recording of stats. Previous stats will remain
///
/// This can be called in a server to revoke consent for recordint stats in that server, 
/// or you can provide a server ID/name via a DM.
pub async fn revoke(ctx: &Context, msg: &Message, mut args: Args) -> CommandResult {
  let id_or_name = if !args.is_empty() {
    Some(args.single_quoted::<String>()?)
  } else {
    None
  };

  let (guild_name, key) = get_name_and_key(ctx, msg, id_or_name).await?;

  let lock = {
    let mut context = ctx.data.write().await;
    context.get_mut::<RedisConnectionKey>()
      .expect("Expected redis connection")
      .clone()
  };

  if let Err(error) = { 
    let mut redis_clinet = lock.lock().await;
    redis_clinet.0.hset::<&str, &str, u64, u64>(&key, "consent", 0).await
  } {
    return handle_command_err(ctx, msg, &error.to_string()).await;
  };


  let _ = msg.author.dm(&ctx.http, |m| {
    m.content(
      &format!("You have revoked consent to record stats of your reactions\nin {}", guild_name))
  }).await?;

  Ok(())
}

#[command]
#[max_args(2)]
#[usage("optional_emoji_limit optional_server_name_id")]
#[example("1000")]
#[example("10 000000000000000000")]
#[example("10 \"test server\"")]
#[example("")]
/// Get stats of your emoji usage in a guild. These stats are DMed
///
/// By default, will send you the top 10 emojis (potentially more if many are tied).
// You can change the number of emojis by providing a number as your first argument
/// 
/// This can be called in a server to get stats for that server, 
/// or you can provide a server ID/name via a DM. 
/// You have to provide an emoji count in this case.
pub async fn stats(ctx: &Context, msg: &Message, mut args: Args) -> CommandResult {
  let emoji_limit = if !args.is_empty() {
    args.single_quoted::<u64>()?
  } else {
    DEFAULT_EMOJI_LIMIT
  };

  let id_or_name = if args.len() == 2 {
    Some(args.single_quoted::<String>()?)
  } else {
    None
  };

  let (guild_name, key) = get_name_and_key(ctx, msg, id_or_name).await?;

  let lock = {
    let mut context = ctx.data.write().await;
    context.get_mut::<RedisConnectionKey>()
      .expect("Expected redis connection")
      .clone()
  };

  let message = {
    let react_stats: HashMap<String, u64> = {
      let mut conn = lock.lock().await;
      match conn.0.hgetall(key).await {
        Ok(res) => res,
        Err(error) => return handle_command_err(ctx, msg, &error.to_string()).await
      }
    };

    if react_stats.len() <= 1{
      String::from("You have used no emojis")
    } else {
      let mut message = String::from(">>> ");
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
  
      let mut emoji_count = 0u64;
  
      for (count, emojis) in score_mapping.iter().rev() {
        let emoji_len = emojis.len() as u64;
  
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

  let _ = msg.author.dm(&ctx.http, |m| {
    m.content(&format!("{}\nin {}", message, guild_name))
  }).await;

  Ok(())
}

#[command]
#[min_args(1)]
#[usage("emojis optional_server_id_name")]
#[example("joy 3️⃣")]
#[example("joy 3️⃣ 4️⃣ 5️⃣")]
#[example("joy 3️⃣ \"test server\"")]
#[example("joy 3️⃣ 000000000000000000")]
/// Get stats of your specific emojis in a server.
///
/// You can provide a list of emojis you want to see.
/// If you want to specify which server to use, provide the server id or name
/// as the last argument.
pub async fn uses(ctx: &Context, msg: &Message, args: Args) -> CommandResult {
  let mut emojis: Vec<&str> = args.raw_quoted().collect();

  let id_or_name: Option<String> = match get_guild_from_id(ctx, emojis.last().unwrap()).await {
    Some(_) => {
      Some(String::from(emojis.pop().unwrap()))
    },
    None => None
  };

  let (guild_name, key) = get_name_and_key(ctx, msg, id_or_name).await?;

  let lock = {
    let mut context = ctx.data.write().await;
    context.get_mut::<RedisConnectionKey>()
      .expect("Expected redis connection")
      .clone()
  };

  let message = {
    let react_stats: HashMap<String, u64> = {
      let mut conn = lock.lock().await;
      match conn.0.hgetall(key).await {
        Ok(res) => res,
        Err(error) => return handle_command_err(ctx, msg, &error.to_string()).await
      }
    };

    if react_stats.len() <= 1 {
      String::from("You have used no emojis")
    } else {
      let mut message = String::from(">>> ");

      for emoji in emojis.iter() {
        let count = react_stats.get(*emoji).unwrap_or(&0);
  
        message += &format!("{}: {} use", emoji, count);
  
        if count != &1 {
          message += "s";
        }
  
        message += "\n";
      }
  
      message
    }
  };

  let _ = msg.author.dm(&ctx.http, |m| {
    m.content(&format!("{}\nin {}", message, guild_name))
  }).await;

  Ok(())
}

async fn get_name_and_key(ctx: &Context, msg: &Message, 
  id_or_name: Option<String>) -> Result<(String, String), CommandError> {
  match id_or_name {
    Some(identifier) => {
      match get_guild_from_id(ctx, &identifier).await {
        Some(guild) => {
          Ok((guild.name.clone(), format!("{}:{}", msg.author.id.0, guild.id.0)))
        },
        None => 
          return command_err!(format!("Could not find a server {}", identifier))
      }
    },
    None => {
      match msg.channel_id.to_channel(&ctx.http).await {
        Ok(c) => match c {
          Channel::Guild(_) => (),
          _ => {
            return command_err!("You must provide a guild id/name. Alternatively, you can message in a server channel")
          }
        },
        Err(err) => return command_err!(err.to_string())
      };


      match get_guild(ctx, msg).await {
        Ok(guild) => {
            Ok((guild.name.clone(), format!("{}:{}", msg.author.id.0, guild.id.0)))
        }
        Err(error) => return Err(error)
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
