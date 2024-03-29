use owoify_rs::{Owoifiable, OwoifyLevel};
use serenity::{
  builder::CreateApplicationCommands,
  model::{
    application::{command::*, interaction::application_command::*},
    prelude::interaction::InteractionResponseType,
  },
  prelude::*,
};

pub fn owo_command(commands: &mut CreateApplicationCommands) -> &mut CreateApplicationCommands {
  commands.create_application_command(|command| command.name("owo").kind(CommandType::Message))
}

pub async fn interaction_owo(
  ctx: &Context,
  interaction: &ApplicationCommandInteraction,
) -> Result<(), String> {
  for msg in interaction.data.resolved.messages.values() {
    let new_content = msg.content.owoify(OwoifyLevel::Owo);

    let _ = interaction
      .create_interaction_response(ctx, |resp| {
        resp
          .kind(InteractionResponseType::ChannelMessageWithSource)
          .interaction_response_data(|msg| msg.content(new_content).ephemeral(true))
      })
      .await;

    return Ok(());
  }

  Err(String::from("No message selected"))
}
