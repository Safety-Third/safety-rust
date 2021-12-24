use redis::{Commands, pipe, RedisError, transaction};
use serenity::prelude::*;
use serenity::model::prelude::*;
use serenity::framework::standard::{
  Args, CommandError, CommandResult, macros::command
};

use std::{
  collections::{BTreeMap, HashMap}, io::{Error, ErrorKind::Other}
};

use crate::util::scheduler::RedisConnectionKey;
use super::{
  util::{EMOJI_REGEX, get_guild, handle_command_err}
};

macro_rules! redis_error {
  ($message:expr) => {
      Err(RedisError::from(Error::new(Other, $message)))
  };
}

const DEFAULT_EMOJI_LIMIT: u64 = 5;

#[command]
#[max_args(2)]
#[usage("optional_emoji_limit optional_server_name_id")]
#[example("20")]
#[example("5 000000000000000000")]
#[example("10 test server")]
#[example("")]
/// Get stats of your emoji usage in a guild, by category. These stats are DMed.
///
/// By default, will send you the top 5 emojis per section (up to 2x max_per_category, if tied).
/// You can change the number of emojis by providing a number as your first argument.
/// Regardless, you will get a total count of the emojis per section.
///
/// This can be called in a server to get stats for that server, 
/// or you can provide a server ID/name via a DM. 
/// You have to provide an emoji count in this case.
pub async fn categories(ctx: &Context, msg: &Message, mut args: Args) -> CommandResult {
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

  let idx = match key.find(":") {
    Some(i) => i,
    None => return handle_command_err(ctx, msg, "Unexpected error").await
  };

  let message = {
    let guild_id = &key[idx + 1..];
    
    let (cats, emojis): (HashMap<String, String>, HashMap<String, u64>) = {
      let mut conn = lock.lock().await;
      
      let result = pipe()
        .hgetall(&format!("{}:categories", guild_id))
        .hgetall(key)
        .query(&mut conn.0);

      match result {
        Ok(res) => res,
        Err(err) => return command_err!(err.to_string())
      }
    };

    
    if emojis.is_empty() {
      String::from("You have used no emojis")
    } else {
      let mut message = String::from(">>> ");

      let mut score_mapping_by_category: HashMap<&str, BTreeMap<u64, Vec<&str>>> = HashMap::new();
    
      for (emoji, count) in emojis.iter() {
        if emoji == "consent" {
          continue;
        }

        let mut category = "**No category**";

        for (key, emojilist) in cats.iter() {
          if emojilist.contains(emoji) {
            category = key;
            break;
          }
        }

        match score_mapping_by_category.get_mut(&category) {
          Some(map) => {
            match map.get_mut(count) {
              Some(existing) => {
                existing.push(emoji);
              },
              None => {
                map.insert(*count, vec![emoji]);
              }
            }
          },
          None => {
            let mut map: BTreeMap<u64, Vec<&str>> = BTreeMap::new();
            map.insert(*count, vec![emoji]);
            score_mapping_by_category.insert(category, map);
          }
        };
      }

      let mut section_and_top_emojis: Vec<(u64, Vec<String>, &str)> = vec![];

      for (category, mapping) in score_mapping_by_category.iter_mut() {
        let mut emoji_count = 0u64;
        let mut total_count = 0u64;

        let mut top_emojis: Vec<String> = vec![];

        for (count, emoji_list) in mapping.iter_mut().rev() {
          let len = emoji_list.len() as u64;

          if emoji_count < emoji_limit && emoji_count + len< (2 * emoji_limit) {
            top_emojis.push(
              format!("{} ({})", emoji_list.join(" "), count));
          }

          emoji_count += len;
          total_count += *count * len;
        }

        section_and_top_emojis.push((total_count, top_emojis, category));
      }

      section_and_top_emojis.sort_by(|a, b| b.0.cmp(&a.0));

      for (total_count, emojis, category) in section_and_top_emojis.iter() {
        message += &format!("{} ({} uses): {}\n", category, total_count, &emojis.join(" "));
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
    redis_clinet.0.hset::<&str, &str, u64, u64>(&key, "consent", 1)
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
    redis_clinet.0.del::<&str, u64>(&key)
  } {
    return handle_command_err(ctx, msg, &error.to_string()).await
  }

  let _ = msg.author.dm(&ctx.http, |m| {
    m.content(
      &format!("You deleted all of your stats\nin {}", guild_name))
  }).await;

  Ok(())
}

#[command("deleteCategory")]
#[aliases("delete_category", "del_cat", "delCat")]
#[only_in("guild")]
#[num_args(1)]
#[required_permissions("MANAGE_EMOJIS")]
#[usage("category")]
#[example("joy")]
/// Deletes a specific emoji
/// This function is server-only (no DMing).
pub async fn delete_category(ctx: &Context, msg: &Message, mut args: Args) -> CommandResult {
  let guild_id = match msg.channel_id.to_channel(&ctx.http).await {
    Ok(channel) => {
      match channel {
        Channel::Guild(guild_channel) => {
          guild_channel.guild_id
        },
        _ => return handle_command_err(ctx, msg, "This command can only be processed in a server").await
      }
    }, 
    Err(_) => return handle_command_err(ctx, msg, "No channel found").await
  };

  let category = args.single_quoted::<String>()?;

  let lock = {
    let mut context = ctx.data.write().await;
    context.get_mut::<RedisConnectionKey>()
      .expect("Expected redis connection")
      .clone()
  };

  {
    let mut conn = lock.lock().await;
    if let Err(error) = conn.0.hdel::<&str, &str, u64>(
      &format!("{}:categories", guild_id.0), &category) {
      return command_err!(error.to_string());
    }
    
  };

  let _ = msg.channel_id.say(&ctx.http, format!(
    "{:?} deleted category {}", msg.author.mention(), &category)).await;

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

  if let Err(error) =     { 
    let mut redis_clinet = lock.lock().await;
    redis_clinet.0.hset::<&str, &str, u64, u64>(&key, "consent", 0)
  } {
    return handle_command_err(ctx, msg, &error.to_string()).await;
  };


  let _ = msg.author.dm(&ctx.http, |m| {
    m.content(
      &format!("You have revoked consent to record stats of your reactions\nin {}", guild_name))
  }).await?;

  Ok(())
}

#[command("setCategory")]
#[aliases("set_category", "set_cat", "setCat")]
#[min_args(2)]
#[only_in("guild")]
#[required_permissions("MANAGE_EMOJIS")]
#[usage("space_separated_emoji_ist")]
#[example("joy 3️⃣")]
#[example("joy 3️⃣ 4️⃣ 5️⃣")]
/// Sets a list of emojis to a specific category.
/// This category will be used to determine which "categories" are most used by a specific person.
/// An emoji can only belong to a single category.
/// This function is server-only (no DMing).
pub async fn set_category(ctx: &Context, msg: &Message, mut args: Args) -> CommandResult {
  let guild_id = match msg.channel_id.to_channel(&ctx.http).await {
    Ok(channel) => {
      match channel {
        Channel::Guild(guild_channel) => {
          guild_channel.guild_id
        },
        _ => return handle_command_err(ctx, msg, "This command can only be processed in a server").await
      }
    }, 
    Err(_) => return handle_command_err(ctx, msg, "No channel found").await
  };

  let key = format!("{}:categories", guild_id.0);
  
  let lock = {
    let mut context = ctx.data.write().await;
    context.get_mut::<RedisConnectionKey>()
      .expect("Expected redis connection")
      .clone()
  };

  let category = args.single_quoted::<String>()?;

  let mut emojis: Vec<String> = Vec::new();

  for emoji in args.iter::<String>() {
    match emoji {
      Ok(em) => {
        if EMOJI_REGEX.is_match(&em) {
          emojis.push(em);
        } else {
          return command_err!(format!("\"{}\" is not a valid emoji", &em));
        }
      },
      Err(error) => return command_err!(error.to_string())
    };
  };

  let _: () = transaction(&mut lock.lock().await.0, &[&key], |conn, pipe| {
    let categories: HashMap<String, String> = conn.hgetall(&key)?;

    for (existing_category, emoji_list) in categories.iter() {
      if category == *existing_category {
        continue;
      }

      for emoji in emojis.iter() {
        if emoji_list.contains(emoji) {
          return redis_error!(
            format!("Emoji {} is already used in category {}", emoji, existing_category));
        }
      }
    }

    pipe
      .hset(&key, &category, emojis.join(" ")).ignore()
      .query(conn)
  })?;

  let _ = msg.channel_id.say(&ctx.http, 
    format!("{} set category {} to {}", msg.author.mention(), &category, &emojis.join(" "))).await;

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
      match conn.0.hgetall(key) {
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
      match conn.0.hgetall(key) {
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

#[command("viewCategories")]
#[aliases("view_categories", "view_cats", "viewCats")]
#[max_args(1)]
#[usage("optional_server_id_name")]
#[example("\"test server\"")]
#[example("000000000000000000")]
/// View the emoji categories in a specific server.
///
/// You can provide a server id or name to specify a certain server to use, or
/// send the message in a server (no arguments) to get the categories for that server.
pub async fn view_categories(ctx: &Context, msg: &Message, mut args: Args) -> CommandResult {
  let (name, id) = if !args.is_empty() {
    let guild_id_or_name = args.single_quoted::<String>()?;

    match get_guild_from_id(ctx, &guild_id_or_name).await {
      Some(guild) => {
        (guild.name.clone(), guild.id.0)
      },
      None => return 
        handle_command_err(ctx, msg, &format!("Could not find a server {}", guild_id_or_name)).await
    }
  } else {
    let result =  get_guild(ctx, msg).await?;
    (result.name, result.id.0)
  };

  let lock = {
    let mut context = ctx.data.write().await;
    context.get_mut::<RedisConnectionKey>()
      .expect("Expected redis connection")
      .clone()
  };

  let categories: HashMap<String, String> = {
    let mut client = lock.lock().await;
    match client.0.hgetall(format!("{}:categories", id)) {
      Ok(result) => result,
      Err(error) => return command_err!(error.to_string())
    }
  };

  if categories.is_empty() {
    let _ = msg.channel_id.say(&ctx.http, &format!("No categories for {}", name)).await;
  } else {
    let mut message = format!(">>> Emoji categories in {}", name);

    for (category, emojilist) in categories.iter() {
      message += &format!("\n{}: {}", category, emojilist);
    }

    let _ = msg.channel_id.say(&ctx.http, &message).await;
  }

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
