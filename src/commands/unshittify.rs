use serenity::{
  builder::CreateApplicationCommands,
  model::{
    application::{command::*, interaction::application_command::*},
    prelude::interaction::InteractionResponseType,
  },
  prelude::*,
};

pub fn unshittify_command(
  commands: &mut CreateApplicationCommands,
) -> &mut CreateApplicationCommands {
  commands
    .create_application_command(|command| command.name("unshitify").kind(CommandType::Message))
}

pub async fn interaciton_unshitify(
  ctx: &Context,
  interaction: &ApplicationCommandInteraction,
) -> Result<(), String> {
  for msg in interaction.data.resolved.messages.values() {
    let new_content = msg
      .content
      .replace("https://twitter.com", "https://vxtwitter.com");

    let (ephermal, new_content) = if new_content == msg.content {
      (true, String::from("No Twitter message found"))
    } else {
      (false, new_content)
    };

    let _ = interaction
      .create_interaction_response(ctx, |resp| {
        resp
          .kind(InteractionResponseType::ChannelMessageWithSource)
          .interaction_response_data(|msg| msg.content(new_content).ephemeral(ephermal))
      })
      .await;

    return Ok(());
  }

  Err(String::from("No message selected"))
}
