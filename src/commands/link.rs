use linkify::{Link, LinkFinder, LinkKind};
use serenity::{
  builder::CreateApplicationCommands, model::prelude::interactions::application_command::*,
  model::prelude::*, prelude::*,
};
use url::Url;

pub fn sanitize_command(
  commands: &mut CreateApplicationCommands,
) -> &mut CreateApplicationCommands {
  commands.create_application_command(|command| {
    command
      .name("sanitize")
      .kind(ApplicationCommandType::Message)
  })
}

pub async fn interaction_sanitize(
  ctx: &Context,
  interaction: &ApplicationCommandInteraction,
) -> Result<(), String> {
  let finder = LinkFinder::new();
  for msg in interaction.data.resolved.messages.values() {
    let links: Vec<Link> = finder
      .links(&msg.content)
      .filter(|link| link.kind() == &LinkKind::Url)
      .collect();

    if links.len() == 0 {
      let _ = interaction
        .create_interaction_response(ctx, |resp| {
          resp
            .kind(InteractionResponseType::ChannelMessageWithSource)
            .interaction_response_data(|msg| {
              msg
                .content("No links were found in this message")
                .flags(InteractionApplicationCommandCallbackDataFlags::EPHEMERAL)
            })
        })
        .await;
    } else {
      let mut any_changed = false;
      let mut message = String::from("Your URLs without tracking params\n>>> ");

      for link in links {
        let mut url = match Url::parse(link.as_str()) {
          Ok(res) => res,
          Err(error) => return Err(error.to_string()),
        };

        let mut changed = false;

        let pairs: Vec<_> = url
          .query_pairs()
          .filter_map(|pair| {
            if pair.0.starts_with("utm") {
              changed = true;
              None
            } else {
              Some(format!("{}={}", pair.0, pair.1))
            }
          })
          .collect();

        if changed {
          any_changed = true;
          url.set_query(Some(&pairs.join("&")));

          message += &format!("{}\n", url);
        }
      }

      if any_changed {
        let _ = interaction
          .create_interaction_response(ctx, |resp| {
            resp
              .kind(InteractionResponseType::ChannelMessageWithSource)
              .interaction_response_data(|msg| msg.content(message))
          })
          .await;
      } else {
        let _ = interaction
          .create_interaction_response(ctx, |resp| {
            resp
              .kind(InteractionResponseType::ChannelMessageWithSource)
              .interaction_response_data(|msg| {
                msg
                  .content("All URLs have already been sanitized")
                  .flags(InteractionApplicationCommandCallbackDataFlags::EPHEMERAL)
              })
          })
          .await;
      }
    }
  }

  Ok(())
}
