use redis::{cmd, AsyncCommands};
use serenity::{
  builder::CreateApplicationCommands, model::prelude::interactions::application_command::*,
  model::prelude::*, prelude::*,
};

use crate::util::scheduler::RedisConnectionKey;

pub fn copy_command(commands: &mut CreateApplicationCommands) -> &mut CreateApplicationCommands {
  commands.create_application_command(|command| {
    command.name("copy").kind(ApplicationCommandType::Message)
  })
}

pub async fn interaction_copy(
  ctx: &Context,
  interaction: &ApplicationCommandInteraction,
) -> Result<(), String> {
  println!(
    "{} {:?} {:?}",
    interaction.user.id.0, interaction.guild_id, interaction.data
  );
  let message = interaction.data.resolved.messages.values().next();

  if message.is_some() && interaction.guild_id.is_some() {
    let message = message.unwrap();

    let contents = &message.content;
    let key = format!(
      "{}:{}:copy",
      message.author.id.0,
      interaction.guild_id.as_ref().unwrap().0
    );

    let lock = {
      let mut context = ctx.data.write().await;
      context
        .get_mut::<RedisConnectionKey>()
        .expect("Expected redis connection")
        .clone()
    };

    let old_value: Option<String> = {
      let mut redis_client = lock.lock().await;

      cmd("SET")
        .arg(key)
        .arg(contents)
        .arg("GET")
        .query_async(&mut redis_client.0)
        .await
        .map_err(|err| format!("Error saving message: {}", err.to_string()))?
    };

    let _ = interaction
      .create_interaction_response(ctx, |resp| {
        resp
          .kind(InteractionResponseType::ChannelMessageWithSource)
          .interaction_response_data(|msg| {
            msg.content(format!(
              "{} set the copy for {} to \n> {}\n\nPrior message:\n> {}",
              interaction.user.mention(),
              message.author.mention(),
              contents.replace("\n", "\n> "),
              old_value
                .map(|val| val.replace("\n", "\n> "))
                .unwrap_or(String::from("---"))
            ))
          })
      })
      .await;

    Ok(())
  } else {
    Err(String::from("Missing content/guild id"))
  }
}

pub fn paste_command(commands: &mut CreateApplicationCommands) -> &mut CreateApplicationCommands {
  commands
    .create_application_command(|command| command.name("paste").kind(ApplicationCommandType::User))
}

pub async fn interaction_paste(
  ctx: &Context,
  interaction: &ApplicationCommandInteraction,
) -> Result<(), String> {
  let user = interaction.data.resolved.users.values().next();
  let guild_id = interaction.guild_id;

  if user.is_some() && guild_id.is_some() {
    let user = user.unwrap();
    let guild_id = guild_id.unwrap();

    let key = format!("{}:{}:copy", user.id.0, guild_id.0);

    let lock = {
      let mut context = ctx.data.write().await;
      context
        .get_mut::<RedisConnectionKey>()
        .expect("Expected redis connection")
        .clone()
    };

    let copy: Option<String> = {
      let mut redis_client = lock.lock().await;

      redis_client
        .0
        .get(key)
        .await
        .map_err(|err| format!("Error retrieving message: {}", err.to_string()))?
    };

    match copy {
      Some(copy) => {
        let _ = interaction
          .create_interaction_response(ctx, |resp| {
            resp
              .kind(InteractionResponseType::ChannelMessageWithSource)
              .interaction_response_data(|msg| {
                msg.content(format!("Paste for {}: \n>>> {}", user.name, copy))
              })
          })
          .await;

        Ok(())
      }
      None => Err(String::from("No copy saved")),
    }
  } else {
    Err(String::from("Missing user/guild id"))
  }
}
