use std::sync::Arc;

use reqwest::Client;
use serde::Deserialize;
use serde_json::{Value, json};
use serenity::{
  model::interactions::application_command::ApplicationCommandInteraction,
  model::prelude::*, prelude::*, utils::Colour
};
use tokio::sync::RwLock;

pub struct CatKey;

impl TypeMapKey for CatKey {
  type Value = Arc<RwLock<String>>;
}

#[derive(Debug, Deserialize)]
struct Cat {
  id: String,
  url: String,
  width: u32,
  height: u32
}

pub fn nya_command() -> Value {
  return json!({
    "name": "nya",
    "description": "Get a random cat"
  })
}

pub async fn interaction_nya(ctx: &Context, interaction: &ApplicationCommandInteraction) -> Result<(), String> {

  let key = {
    let data_read = ctx.data.read().await;
    data_read.get::<CatKey>()
      .expect("Expected Cat API key")
      .clone()
      .read()
      .await
      .clone()
  };

  let client = Client::new();
  let response = client.get("https://api.thecatapi.com/v1/images/search")
    .header("x-api-key", key)
    .query(&[("limit", "1"), ("size", "full")])
    .send()
    .await;

  match response {
    Ok(resp) => {
      match resp.json::<Vec<Cat>>().await {
        Ok(cats) => {
          let url = &cats[0].url;
          let _ = interaction.create_interaction_response(&ctx.http, |response| {
            response.kind(InteractionResponseType::ChannelMessageWithSource)
              .interaction_response_data(|msg| msg
                .create_embed(|e| {
                  e
                    .colour(Colour::BLITZ_BLUE)
                    .image(url)
                    .footer(|f| f.text(format!("Source: {}", url)))
                }))
          }).await;
    
          Ok(())
        },
        Err(error) => Err(error.to_string())
      }
    },
    Err(error) => Err(error.to_string())
  }
}