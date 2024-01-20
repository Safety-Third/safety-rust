use serenity::{
  builder::CreateApplicationCommands,
  model::application::{
    command::*,
    interaction::{application_command::*, *},
  },
  prelude::*,
  utils::Color,
};

const DESCRIPTION: &str = "Welcome to Safety-chan v4.
This bot has a few purposes:
1. Make and handle polls.
2. Message out about birthdays
3. Memes: getting cats, owoifying, and rolling dice.
These are summarized in the below fields:";

pub fn help_command(commands: &mut CreateApplicationCommands) -> &mut CreateApplicationCommands {
  commands.create_application_command(|command| {
    command
      .name("help")
      .description("Get a help message")
      .create_option(|new| {
        new
          .name("show_to_all")
          .kind(CommandOptionType::Boolean)
          .description("Whether to make this help message visible to everyone")
          .required(false)
      })
  })
}

pub async fn interaction_help(
  ctx: &Context,
  interaction: &ApplicationCommandInteraction,
) -> Result<(), String> {
  let mut ephemeral = false;

  if interaction.data.options.len() > 0 {
    ephemeral = match &interaction.data.options[0].value {
      Some(field) => match field.as_bool() {
        Some(boolean) => boolean,
        None => return Err(String::from("This must be a true or false")),
      },
      None => return Err(String::from("This must be a true or false")),
    };
  }

  let _ = interaction.create_interaction_response(ctx, |resp|
    resp.kind(InteractionResponseType::ChannelMessageWithSource)
    .interaction_response_data(|msg| {
      msg
        .ephemeral(!ephemeral)
        .embed(|e| {
        e.color(Color::BLITZ_BLUE)
          .title("Safety-chan Help!")
          .description(DESCRIPTION)
            .field("/briefing", "Create a Safety Briefing. This opens a menu for you to post your news, to be send Monday at 7:30 AM Eastern", false)
            .field("/poll new", "Create a new poll, with a set time, topic, and options. You can optionally allow others to add options later, but there is no editing or deleting of options (however, you can delete the entire poll)", false)
            .field("/poll options_add", "Add an option to a poll. You can do this if you are the creator, or the poll is open", false)
            .field("/roll", "Roll one or more dice", false)
            .field("/nya", "Get a cat", false)
            .field("/stats consent *", "This allows you to approve, delete, or revoke collecting of your emoji usage in a given server", false)
            .field("/stats get", "If you have consented to collecting emoji stats, get your stats in a DM", false)
            .field("owo", "You can click on a message to see it owo-ified. I am not responsible for damages", false)
            .field("sanitize", "You can click on a message with a link, and it will strip out utm_ tracking", false)
        })
      })
  ).await;

  Ok(())
}
