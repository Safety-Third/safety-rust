mod commands;
mod util;

use std::{
  env::var,
  ops::{Add, Sub},
  sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
  },
  time::Duration,
};

use chrono::{Datelike, Duration as ChronoDuration, Timelike, Utc};
use chrono_tz::EST5EDT;
use redis::{AsyncCommands, Client};
use serenity::{
  async_trait,
  client::Client as DiscordClient,
  http::Http,
  model::{
    gateway::{Activity, GatewayIntents},
    id::{ChannelId, GuildId},
    interactions::{application_command::ApplicationCommand, Interaction, InteractionResponseType},
  },
  prelude::{Context, EventHandler},
};
use tokio::{
  spawn,
  sync::{Mutex, RwLock},
  time::{interval, sleep},
};

use commands::{help::*, link::*, news::*, nya::*, owo::*, poll::*, roll::*};

use util::{
  rng::random_number,
  scheduler::{
    Callable, MyVec, RedisConnectionKey, RedisSchedulerKey, RedisWrapper,
    Scheduler as RedisScheduler,
  },
  sheets::{parse_date, query},
};

struct Handler {
  loop_running: AtomicBool,
}

#[async_trait]
impl EventHandler for Handler {
  async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
    match interaction {
      Interaction::ApplicationCommand(app_command) => {
        let command_name = app_command.data.name.as_str();
        if let Err(error) = match command_name {
          "briefing" => interaction_briefing(&ctx, &app_command).await,
          "help" => interaction_help(&ctx, &app_command).await,
          "nya" => interaction_nya(&ctx, &app_command).await,
          "owo" => interaction_owo(&ctx, &app_command).await,
          "poll" => interaction_poll(&ctx, &app_command).await,
          "roll" => interaction_roll(&ctx, &app_command).await,
          "sanitize" => interaction_sanitize(&ctx, &app_command).await,
          _ => Err(format!("No command {}", command_name)),
        } {
          let _ = app_command
            .create_interaction_response(ctx, |resp| {
              resp
                .kind(InteractionResponseType::ChannelMessageWithSource)
                .interaction_response_data(|msg| {
                  msg
                    .content(format!("An error occurred: {}", error))
                    .ephemeral(true)
                })
            })
            .await;
        }
      }
      Interaction::ModalSubmit(submit) => {
        if let Err(error) = match submit.data.custom_id.as_str() {
          "briefing" => interaction_briefing_followup(&ctx, &submit).await,
          "options_add" => interaction_poll_add_followup(&ctx, &submit).await,
          _ => Err(format!("No modal {}", submit.data.custom_id)),
        } {
          let _ = submit
            .create_interaction_response(ctx, |resp| {
              resp
                .kind(InteractionResponseType::ChannelMessageWithSource)
                .interaction_response_data(|msg| {
                  msg
                    .content(format!("An error occurred: {}", error))
                    .ephemeral(true)
                })
            })
            .await;
        }
      }
      Interaction::MessageComponent(comp_inter) => {
        if let Err(error) = match comp_inter.data.custom_id.as_str() {
          "close" | "delete" => handle_poll_interaction(&ctx, &comp_inter).await,
          "add" => handle_poll_add(&ctx, &comp_inter).await,
          "toggle" => handle_poll_options_toggle(&ctx, &comp_inter).await,
          _ => Ok(()),
        } {
          println!("An error occurred: {:?}", error);
          let _ = comp_inter
            .create_interaction_response(ctx, |resp| {
              resp
                .kind(InteractionResponseType::ChannelMessageWithSource)
                .interaction_response_data(|msg| {
                  msg.content(format!(
                    "Could not process {} request: {}",
                    comp_inter.data.custom_id, error
                  ))
                })
            })
            .await;
        }
      }
      _ => {}
    }
  }

  async fn cache_ready(&self, ctx: Context, _guilds: Vec<GuildId>) {
    if !self.loop_running.load(Ordering::Relaxed) {
      println!("Starting thread");
      let ctx_clone = Arc::new(ctx);

      spawn(async move {
        let statuses = [
          "What is Jeff?",
          "horse plinko",
          "7 blunders",
          "the missile knows",
          "fight me",
        ];

        let mut interval = interval(Duration::from_secs(60 * 60));

        loop {
          interval.tick().await;
          let idx = random_number(statuses.len());
          ctx_clone
            .set_activity(Activity::playing(statuses[idx]))
            .await;
        }
      });

      self.loop_running.swap(true, Ordering::Relaxed);
    }
  }
}

#[tokio::main(flavor = "multi_thread", worker_threads = 12)]
async fn main() {
  let api_key = var("GOOGLE_API_KEY").expect("Expected Google API key");

  let app_id: u64 = var("RUST_APP_ID")
    .expect("Expected an application id in the environment")
    .parse()
    .expect("application id is not a valid id");

  let birthday_announce_channel = var("SAFETY_ANNOUNCEMENT_CHANNEL")
    .expect("Expected birthday announcement channel")
    .parse::<u64>()
    .expect("Expected channel to be a number");

  let birthday_sheet_id = var("SAFETY_GOOGLE_DOCS_LINK").expect("Expected Safety Google Docs link");

  let birthday_vector = vec!["'2018-2019 Roster'!A2:A51", "'2018-2019 Roster'!J2:J51"];

  let redis_url = var("SAFETY_REDIS_URL").unwrap_or_else(|_| String::from("redis://127.0.0.1"));

  let token = &var("RUST_BOT").expect("token");

  let guild_id = var("SAFETY_GUILD_ID")
    .expect("Expected Guild Id")
    .parse::<u64>()
    .expect("Id is not valid");

  let http = Http::new_with_application_id(&token, app_id);

  let intents = GatewayIntents::DIRECT_MESSAGES | GatewayIntents::GUILDS;

  let mut client = DiscordClient::builder(&token, intents)
    .event_handler(Handler {
      loop_running: AtomicBool::new(false),
    })
    .application_id(app_id)
    .await
    .expect("Error creating client");

  let cat_api_key = var("SAFETY_CAT_KEY").expect("Expected an API key for cats");

  {
    let cat_key = Arc::new(RwLock::new(cat_api_key));

    let mut data = client.data.write().await;
    data.insert::<CatKey>(cat_key);
    data.insert::<BriefingGuildKey>(guild_id);
  }

  {
    let http_arc = Arc::new(http);

    let redis_client = Client::open(redis_url).expect("Should be able to create a redis client");

    let connection = redis_client
      .get_async_connection()
      .await
      .expect("Should be able to create a redis connection");

    let persistent_connection = redis_client
      .get_async_connection()
      .await
      .expect("Should be able to create a second redis connection");

    let redis_scheduler = RedisScheduler::new(connection);

    let redis_scheduler_arc = Arc::new(Mutex::new(redis_scheduler));

    let lock = redis_scheduler_arc.clone();

    spawn(async move {
      let mut interval = interval(Duration::from_secs(30));

      loop {
        let jobs = {
          let now = Utc::now().timestamp();
          let mut task_scheduler = lock.lock().await;
          task_scheduler.get_and_clear_ready_jobs(now).await
        };

        let arc_clone = http_arc.clone();

        spawn(async move {
          match jobs {
            Ok(tasks) => {
              for job in tasks.iter() {
                job.call(&arc_clone).await;
              }
            }
            Err(error) => println!("{:?}", error),
          };
        });

        interval.tick().await;
      }
    });

    let http_clone = client.cache_and_http.http.clone();

    spawn(async move {
      loop {
        let now = Utc::now().with_timezone(&EST5EDT);
        let next_time = now.date().succ().and_hms(0, 0, 30);

        let duration_to_sleep = next_time.signed_duration_since(now).to_std().unwrap();

        sleep(duration_to_sleep).await;

        match query(&api_key, &birthday_sheet_id, &birthday_vector).await {
          Ok(sheet) => {
            if sheet.value_ranges.len() != 2 {
              println!("Sheet has more than two ranges: {:?}", sheet);
              return;
            }

            for idx in 0..49 {
              let potential_date = &sheet.value_ranges[1].values[idx];
              let potential_name = &sheet.value_ranges[0].values[idx];

              if potential_date.len() != 1 || potential_name.len() != 1 {
                continue;
              }

              let (month, day, year) = match parse_date(&potential_date[0]) {
                Ok(result) => result,
                Err(_) => continue,
              };

              if next_time.day() == day && next_time.month() == month {
                let msg = format!(
                  "Happy birthday {}! ({} years)!",
                  potential_name[0],
                  next_time.year() - year
                );

                let result = ChannelId(birthday_announce_channel)
                  .say(&http_clone, msg)
                  .await;
                println!("{:?}", result);
              }
            }
          }
          Err(error) => {
            println!("Failed to execute query: {:?}", error);
            return;
          }
        };
      }
    });

    let conn_key = Arc::new(Mutex::new(RedisWrapper(persistent_connection)));
    let conn_clone = conn_key.clone();

    let http_clone2 = client.cache_and_http.http.clone();

    spawn(async move {
      let mut now = Utc::now().with_timezone(&EST5EDT);

      let mut this_weeks_monday = now
        .sub(ChronoDuration::days(
          now.weekday().num_days_from_monday().into(),
        ))
        .with_hour(7)
        .unwrap()
        .with_minute(30)
        .unwrap();

      let current_jobs: MyVec = {
        let mut client = conn_clone.lock().await;
        client
          .0
          .zrangebyscore(BRIEFING_KEY, "-inf", this_weeks_monday.timestamp())
          .await
          .expect("Expected to be able to get keys")
      };
      if current_jobs.v.len() > 0 {
        if let Err(error) =
          send_briefing(&current_jobs, birthday_announce_channel, &http_clone2).await
        {
          println!("{}", error);
        } else {
          let _: u64 = {
            let mut client = conn_clone.lock().await;
            client
              .0
              .zrembyscore(BRIEFING_KEY, "-inf", this_weeks_monday.timestamp())
              .await
              .expect("Expected to be able to remove old keys")
          };
        }
      }

      loop {
        let next_time = this_weeks_monday.add(ChronoDuration::weeks(1));

        let duration_to_sleep = next_time.signed_duration_since(now).to_std().unwrap();

        sleep(duration_to_sleep).await;

        let current_jobs: MyVec = {
          let mut client = conn_clone.lock().await;
          client
            .0
            .zrangebyscore(BRIEFING_KEY, "-inf", next_time.timestamp())
            .await
            .expect("Expected to be able to get keys")
        };
        if current_jobs.v.len() > 0 {
          if let Err(error) =
            send_briefing(&current_jobs, birthday_announce_channel, &http_clone2).await
          {
            println!("{}", error);
          } else {
            let _: u64 = {
              let mut client = conn_clone.lock().await;
              client
                .0
                .zrembyscore(BRIEFING_KEY, "-inf", next_time.timestamp())
                .await
                .expect("Expected to be able to remove old keys")
            };
          }
        }

        this_weeks_monday = next_time;
        now = Utc::now().with_timezone(&EST5EDT);
      }
    });

    {
      let mut data = client.data.write().await;
      data.insert::<RedisSchedulerKey>(redis_scheduler_arc);
      data.insert::<RedisConnectionKey>(conn_key);
    }
  }

  {
    let http = Http::new_with_application_id(&token, app_id);

    GuildId::set_application_commands(&GuildId(guild_id), &http, |commands| commands)
      .await
      .expect("Expected to create guild commands");

    ApplicationCommand::set_global_application_commands(&http, |commands| {
      sanitize_command(roll_command(poll_command(owo_command(nya_command(
        news_command(help_command(commands)),
      )))))
    })
    .await
    .expect("Expected to clear application commands");
  }

  if let Err(why) = client.start().await {
    println!("An error occurred while running the client: {:?}", why);
  }
}
