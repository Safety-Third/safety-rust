use serenity::prelude::*;
use serenity::model::prelude::*;
use serenity::framework::standard::{
  Args, CommandError, CommandResult,
  macros::command,
};

use super::util::*;

#[command]
#[description = "Impersonate this bot in a given channel"]
#[example("000000000000000000 this is a message")]
#[example("\"some-channel\" this is a message")]
#[min_args(2)]
#[owners_only]
#[required_permissions("SEND_MESSAGES")]
#[usage("channel_id message")]
pub fn impersonate(ctx: &mut Context, msg: &Message, mut args: Args) -> CommandResult {
  let guild = get_guild(ctx, msg)?;
  
  let arg_as_channel_id = args.parse::<u64>();

  let channel_id = if arg_as_channel_id.is_ok() {
    ChannelId(args.single::<u64>().unwrap())
  } else {
    let channel_name = args.single_quoted::<String>()?;
    get_channel_from_string(ctx, &guild.read(), &channel_name)?
  };

  let message = match args.remains() {
    Some(text) => text,
    None =>  return Err(CommandError(String::from("You must provide a message")))
  };
  
  match channel_id.say(&ctx.http, format!("```{}```", message)) {
    Ok(_) => Ok(()),
    Err(error) => Err(CommandError(error.to_string()))
  }
}