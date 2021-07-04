use serenity::prelude::*;
use serenity::model::prelude::*;
use serenity::framework::standard::{
  Args, CommandResult, macros::command,
};

use super::util::{get_channel_from_string, get_guild, handle_command_err};

#[command]
#[example("000000000000000000 this is a message")]
#[example("\"some-channel\" this is a message")]
#[min_args(2)]
#[owners_only]
#[required_permissions("SEND_MESSAGES")]
#[usage("channel_id message")]
/// Impersonate this bot in a given channel
/// You must also be present in the channel to message it
pub async fn impersonate(ctx: &Context, msg: &Message, mut args: Args) -> CommandResult {
  let guild = get_guild(ctx, msg).await?;
  
  let arg_as_channel_id = args.parse::<u64>();

  let channel_id = if arg_as_channel_id.is_ok() {
    ChannelId(args.single::<u64>().unwrap())
  } else {
    let channel_name = args.single_quoted::<String>()?;
    get_channel_from_string(&ctx, &guild, &channel_name).await?
  };

  let message = match args.remains() {
    Some(text) => text,
    None => return handle_command_err(ctx, msg, "You must provide a message").await
  };
  
  match channel_id.say(&ctx.http, format!("```{}```", message)).await {
    Ok(_) => Ok(()),
    Err(error) => Err(Box::new(error))
  }
}