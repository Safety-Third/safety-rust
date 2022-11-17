use crate::{MyVec, RedisConnectionKey};
use std::sync::Arc;

use chrono::Utc;
use chrono_tz::EST5EDT;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use serenity::{
  builder::CreateApplicationCommands,
  http::Http,
  model::{
    application::interaction::{application_command::*, *},
    prelude::{
      component::{ActionRowComponent, InputTextStyle},
      interaction::modal::ModalSubmitInteraction,
      ChannelId, Guild,
    },
  },
  prelude::*,
};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Briefing {
  pub author: String,
  pub heading: String,
  pub body: Option<String>,
  pub time: String,
}

pub struct BriefingGuildKey;
impl TypeMapKey for BriefingGuildKey {
  type Value = u64;
}

pub const BRIEFING_KEY: &str = "safety:briefings";

pub fn news_command(commands: &mut CreateApplicationCommands) -> &mut CreateApplicationCommands {
  commands
    .create_application_command(|command| command.name("briefing").description("Start a briefing"))
}

pub async fn interaction_briefing(
  ctx: &Context,
  interaction: &ApplicationCommandInteraction,
) -> Result<(), String> {
  let _ = interaction
    .create_interaction_response(ctx, |response| {
      response
        .kind(InteractionResponseType::Modal)
        .interaction_response_data(|msg| {
          msg
            .ephemeral(true)
            .title("Create a new briefing")
            .custom_id("briefing")
            .components(|comp| {
              comp
                .create_action_row(|row| {
                  row.create_input_text(|text| {
                    text
                      .custom_id("title")
                      .label("Your briefing header!")
                      .placeholder("Something newsworthy *dootdootdootdoot*")
                      .required(true)
                      .style(InputTextStyle::Short)
                      .max_length(100)
                  })
                })
                .create_action_row(|row| {
                  row.create_input_text(|text| {
                    text
                      .custom_id("content")
                      .label("Give us the deets (or don't; that's ok too)")
                      .placeholder("(This is optional)")
                      .required(false)
                      .style(InputTextStyle::Paragraph)
                      .max_length(3800)
                  })
                })
            })
        })
    })
    .await;

  Ok(())
}

pub async fn interaction_briefing_followup(
  ctx: &Context,
  modal: &ModalSubmitInteraction,
) -> Result<(), String> {
  let mut heading: Option<String> = None;
  let mut body: Option<String> = None;

  for row in &modal.data.components {
    for component in &row.components {
      if let ActionRowComponent::InputText(text) = component {
        if text.custom_id == "title" {
          heading = Some(text.value.clone());
        } else if text.custom_id == "content" {
          body = Some(text.value.clone());
        }
      }
    }
  }

  if heading == None {
    return Err(String::from("You must provide a heading"));
  }

  let guild_id = {
    ctx
      .data
      .read()
      .await
      .get::<BriefingGuildKey>()
      .expect("Expected guild id")
      .clone()
  };

  match Guild::get(&ctx, guild_id).await {
    Ok(guild) => {
      if let Err(_) = guild.member(&ctx, modal.user.id).await {
        return Err(String::from("You are not approved"));
      }
    }
    Err(error) => return Err(error.to_string()),
  }

  let now = Utc::now().with_timezone(&EST5EDT);

  let briefing = Briefing {
    author: modal.user.mention().to_string(),
    heading: heading.unwrap().to_string(),
    body: body,
    time: now.format("%A %B %-d, %Y %-I:%M %P").to_string(),
  };

  let serialized = match bincode::serialize(&briefing) {
    Ok(data) => data,
    Err(_) => {
      return Err(String::from(
        "Could not serialize your briefing. Please but bot manager",
      ));
    }
  };

  let result = {
    let lock = {
      let mut context = ctx.data.write().await;
      context
        .get_mut::<RedisConnectionKey>()
        .expect("Expected redis connection")
        .clone()
    };

    let mut redis_client = lock.lock().await;
    redis_client
      .0
      .zadd::<&str, i64, Vec<u8>, u64>(BRIEFING_KEY, serialized, now.timestamp())
      .await
  };

  match result {
    Ok(_) => {
      let _ = modal
        .create_interaction_response(ctx, |resp| {
          resp
            .kind(InteractionResponseType::ChannelMessageWithSource)
            .interaction_response_data(|msg| msg.content("Submitted").ephemeral(true))
        })
        .await;
    }
    Err(error) => return Err(format!("Could not save your briefing: {:?}", error)),
  };

  Ok(())
}

pub async fn send_briefing(
  briefings: &MyVec,
  channel_id: u64,
  http: &Arc<Http>,
) -> Result<(), String> {
  let mut messages: Vec<String> = vec![];
  let mut current_message = String::from("");

  for briefing in &briefings.v {
    let brief: Briefing = match bincode::deserialize(&briefing) {
      Ok(data) => data,
      Err(error) => return Err(format!("Could not deserialize briefing: {:?}", error)),
    };

    let next_message = match brief.body {
      Some(ref body) => format!(
        concat!("**{}**\n", "```{}```", " - {} {}\n\n",),
        brief.heading, body, brief.author, brief.time
      ),
      None => format!(
        concat!("**{}**\n", " - {} {}\n\n"),
        brief.heading, brief.author, brief.time
      ),
    };

    if current_message.len() + next_message.len() < 2000 {
      current_message += &next_message;
    } else {
      messages.push(current_message);
      current_message = next_message;
    }
  }

  let header = String::from("*DOOTDOOTDOOTDOOT*. It's time for your Safety Weekly Briefing!");

  messages.push(current_message);
  let channel = ChannelId(channel_id);

  for message in messages {
    if let Err(error) = channel
      .send_message(http, |m| {
        m.embed(|e| e.title(header.clone()).description(message))
      })
      .await
    {
      return Err(format!("Could not send briefing: {:?}", error));
    }
  }

  Ok(())
}
