
use serenity::prelude::*;
use serenity::model::prelude::*;
use serenity::framework::standard::{
  CommandError,
};
use serenity::utils::parse_mention;

use parking_lot::RwLock;
use std::sync::Arc;

#[macro_export]
macro_rules! error {
  ($type:expr, $value:expr) => {
    Err(CommandError(format!("Could not find {} {}", $type, $value))) 
  };
}

pub fn get_channel_from_string(ctx: &mut Context, 
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
      match guild.channel_id_from_name(&ctx.cache, &name_or_id) {
        Some(id) => Ok(id),
        None =>  error!("channel", name_or_id)
      }
    }
  }
}

pub fn get_guild(ctx: &mut Context, msg: &Message) 
  -> Result<Arc<RwLock<Guild>>, CommandError> {
  match msg.guild(&ctx.cache) {
    Some(guild) => Ok(guild),
    None => Err(CommandError(String::from("Guild not in cache")))
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
        Some(member) => Ok(member.user_id()),
        None => error!("user", name_or_id)
      }
    }
  }
}
