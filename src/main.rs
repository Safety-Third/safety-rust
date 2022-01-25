mod commands;
mod util;

use std::{
  collections::{HashMap, HashSet}, env::var,
  sync::{atomic::{AtomicBool, Ordering}, Arc}, time::Duration
};

use chrono::{Datelike, Utc};
use chrono_tz::EST5EDT;
use redis::{AsyncCommands, Client, RedisResult};
use serde_json::json;
use serenity::{
  async_trait,
  client::{Client as DiscordClient, bridge::gateway::GatewayIntents},
  framework::standard::{
    Args, CommandGroup, CommandResult, Delimiter, DispatchError,
    HelpOptions, help_commands, StandardFramework,
    macros::{group, help, hook}
  },
  http::Http,
  model::{
    channel::{Message, Reaction},
    gateway::Activity, id::{ChannelId, GuildId, UserId},
    interactions::{
      Interaction, InteractionResponseType
    }
  },
  prelude::{Context,EventHandler}
};
use tokio::{spawn, sync::{Mutex, RwLock}, time::{interval, sleep}};

use commands::{
  nya::*,
  poll::*,
  roles::*,
  roll::*,
  stats::*,
  util::{EMOJI_REGEX, get_guild}
};

use util::{
  rng::random_number,
  scheduler::{
    Callable, RedisSchedulerKey,
    RedisConnectionKey, RedisWrapper, Scheduler as RedisScheduler
  },
  sheets::{parse_date, query},
};


#[group]
#[commands(add_roles, remove_roles, set_roles)]
#[description = "Commands for managing roles (admin-only)"]
struct Roles;

#[group]
#[commands(consent, delete, revoke, stats, uses)]
#[description = "Manage and view emoji stats"]
struct Stats;

struct Handler {
  loop_running: AtomicBool
}

#[async_trait]
impl EventHandler for Handler {
  async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
    match interaction {
      Interaction::ApplicationCommand(app_command) => {
        if let Err(error) = match app_command.data.name.as_str() {
          "nya" => interaction_nya(&ctx, &app_command).await,
          "poll" => interaction_poll(&ctx, &app_command).await,
          "roll" => interaction_roll(&ctx, &app_command).await,
          "stats" => interaction_stats_entrypoint(&ctx, &app_command).await,
          _ => Ok(())
        } {
          let _ = app_command.create_interaction_response(&ctx.http, |resp|
            resp.kind(InteractionResponseType::ChannelMessageWithSource)
            .interaction_response_data(|msg|
              msg.content(format!("An error occurred: {}", error)))).await;
        }
      },
      Interaction::MessageComponent(comp_inter) => {
        let msg = &comp_inter.message;
        let message = if msg.mentions.len() == 1 && comp_inter.user == msg.mentions[0] {
          Some(msg)
        } else {
          None
        };

        if let Some(msg) = message {
          if let Err(error) = msg.delete(&ctx.http).await {
            let _ = comp_inter.create_interaction_response(&ctx.http, |resp| {
              resp.kind(InteractionResponseType::ChannelMessageWithSource)
                .interaction_response_data(|msg| msg
                  .content(format!("Could not delete poll: {}", error)))
            });
          } else {
            let lock = {
              let mut context = ctx.data.write().await;
              context.get_mut::<RedisSchedulerKey>()
                .expect("Expected redis instance")
                .clone()
            };

            {
              let mut redis_scheduler = lock.lock().await;
              let _ = redis_scheduler.remove_job(&msg.embeds[0].fields[1].value).await;
            };
          }
        } else {
          let _ = comp_inter.create_interaction_response(&ctx.http, |resp| {
            resp.kind(InteractionResponseType::DeferredUpdateMessage)
          }).await;
        };
      },
      _ => {}
    }
  }

  async fn message(&self, ctx: Context, msg: Message) {
    if msg.author.bot {
      return;
    }

    let id = match msg.guild_id {
      Some(i) => i.0,
      None => {
        match get_guild(&ctx, &msg).await {
          Ok(guild) => guild.id.0,
          Err(_) => return
        }
      }
    };

    let key = format!("{}:{}", msg.author.id.0, id);

    let lock = {
      let mut context = ctx.data.write().await;
      context.get_mut::<RedisConnectionKey>()
        .expect("Expected redis instance")
        .clone()
    };

    let mut data: HashMap<String, u64> = {
      let mut redis_client = lock.lock().await;
      match redis_client.0.hgetall(&key).await {
        Ok(result) => result,
        Err(_) => HashMap::new()
      }
    };

    if data.get("consent") != Some(&1) {
      return;
    }

    for mat in EMOJI_REGEX.find_iter(&msg.content) {
      if mat.end() > mat.start() + 1 {
        let key = &msg.content[mat.range()];
        let new_value = match data.get(key){
          Some(existing) => existing + 1,
          None => 1
        };

        data.insert(key.to_owned(), new_value);
      }
    }

    let mut items: Vec<(String, u64)> = vec![];

    for (key, val) in data.into_iter() {
      items.push((key, val));
    }

    {
      let mut redis_client = lock.lock().await;
      let res: RedisResult<String> = redis_client.0.hset_multiple(key, &items).await;
      if let Err(error) = res {
        println!("{:?}", error);
      }
    }
  }

  async fn reaction_add(&self, ctx: Context, reaction: Reaction) {
    let guild_id = match reaction.guild_id {
      Some(id) => id,
      None => return
    };

    let user = match reaction.user(&ctx.http).await {
      Ok(u) => u,
      Err(_) => return
    };

    if user.bot {
      return;
    }

    let user_id = match reaction.user_id {
      Some(id) => id,
      None => return
    };

    let key = format!("{}:{}", user_id.0, guild_id);

    let lock = {
      let mut context = ctx.data.write().await;
      context.get_mut::<RedisConnectionKey>()
        .expect("Expected redis instance")
        .clone()
    };

    let data: HashMap<String, u64> = {
      let mut redis_client = lock.lock().await;

      match redis_client.0.hgetall(&key).await {
        Ok(result) => result,
        Err(_) => HashMap::new()
      }
    };

    if data.get("consent") != Some(&1) {
      return
    }

    let emoji_str = format!("{}", reaction.emoji);

    let new_data = match data.get(&emoji_str) {
      Some(old) => old + 1,
      None => 1
    };

    {
      let mut redis_client = lock.lock().await;
      let res: RedisResult<u64> = redis_client.0.hset(&key, emoji_str, new_data).await;

      if let Err(error) = res {
        println!("{:?}", error);
      }
    };
  }

  async fn reaction_remove(&self, ctx: Context, reaction: Reaction) {
    let guild_id = match reaction.guild_id {
      Some(id) => id,
      None => return
    };

    let user = match reaction.user(&ctx.http).await {
      Ok(u) => u,
      Err(_) => return
    };

    if user.bot {
      return;
    }

    let user_id = match reaction.user_id {
      Some(id) => id,
      None => return
    };

    let key = format!("{}:{}", user_id.0, guild_id);

    let lock = {
      let mut context = ctx.data.write().await;
      context.get_mut::<RedisConnectionKey>()
        .expect("Expected redis instance")
        .clone()
    };

    let data: HashMap<String, u64> = {
      let mut redis_client = lock.lock().await;
      match redis_client.0.hgetall(&key).await {
        Ok(result) => result,
        Err(_) => HashMap::new()
      }
    };

    if data.get("consent") != Some(&1) {
      return;
    }

    let emoji_str = format!("{}", reaction.emoji);

    let new_data = match data.get(&emoji_str) {
      Some(old) => old - 1,
      None => 0
    };

    {
      let mut redis_client = lock.lock().await;

      let res: RedisResult<u64> = {
        if new_data != 0 {
          redis_client.0.hset(&key, emoji_str, new_data).await
        } else {
          redis_client.0.hdel(&key, emoji_str).await
        }
      };

      if let Err(error) = res {
        println!("{:?}", error);
      }
    };
  }

  async fn cache_ready(&self, ctx: Context, _guilds: Vec<GuildId>) {
    let statuses = [
      "What is Jeff?", "horse plinko", "7 blunders", "the missile knows", "fight me"
    ];

    if !self.loop_running.load(Ordering::Relaxed) {
      let ctx_clone = Arc::new(ctx);

      spawn(async move {
        let mut interval = interval(Duration::from_secs(60 * 60));

        loop {
          interval.tick().await;
          let idx = random_number(statuses.len());
          ctx_clone.set_activity(Activity::playing(statuses[idx])).await;
        };
      });

      self.loop_running.swap(true, Ordering::Relaxed);
    }
  }
}

#[help]
#[individual_command_tip = "Welcome to Bot v3.\n\
For help on a specific command, just pass that name in.
To see this in embed form, pass `embed` as your first option (e.g. `help embed other_stuff`)"]
#[command_not_found_text = "Could not find: `{}`."]
#[max_levenshtein_distance(3)]
#[lacking_permissions = "Hide"]
#[lacking_role = "Nothing"]
#[wrong_channel = "Strike"]
async fn my_help(
  context: &Context,
  msg: &Message,
  args: Args,
  help_options: &'static HelpOptions,
  groups: &[&'static CommandGroup],
  owners: HashSet<UserId>
) -> CommandResult {
  let embed = args.current() == Some("embed");

  if embed {
    let message = args.message();
    let len_to_remove = std::cmp::min(6, message.len());

    let remaining_args = Args::new(&message[len_to_remove..], &[Delimiter::Single(' ')]);
    help_commands::with_embeds(context, msg, remaining_args, help_options, groups, owners).await;
  } else {
    help_commands::plain(context, msg, args, help_options, groups, owners).await;
  }
  Ok(())
}

#[hook]
async fn after(ctx: &Context, msg: &Message, _name: &str, result: CommandResult) {
  if !msg.author.bot {
    if let Err(error) = result {
      let _ = msg.channel_id.say(&ctx.http,
        &format!("Error in {:?}:\n{}", msg.content, error)).await;
    }
  }
}

#[hook]
async fn dispatch_error(ctx: &Context, msg: &Message, error: DispatchError) {
  if !msg.author.bot {
    let _ = msg.channel_id.say(&ctx.http,
      &format!("Error in {:?}:\n{:?}", msg.content, error)).await;
  }
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
  let api_key = var("GOOGLE_API_KEY")
    .expect("Expected Google API key");

  let app_id: u64 = var("RUST_APP_ID")
    .expect("Expected an application id in the environment")
    .parse().expect("application id is not a valid id");

  let birthday_announce_channel = var("SAFETY_ANNOUNCEMENT_CHANNEL")
    .expect("Expected birthday announcement channel")
    .parse::<u64>()
    .expect("Expected channel to be a number");

  let birthday_sheet_id = var("SAFETY_GOOGLE_DOCS_LINK")
    .expect("Expected Safety Google Docs link");

  let birthday_vector = vec!("'2018-2019 Roster'!A2:A51", "'2018-2019 Roster'!J2:J51");

  let redis_url = var("SAFETY_REDIS_URL")
    .unwrap_or_else(|_| String::from("redis://127.0.0.1"));

  let token = &var("RUST_BOT").expect("token");


  let http = Http::new_with_token(&token);

  let owners = {
    match http.get_current_application_info().await {
      Ok(info) => {
        let mut owners = HashSet::new();
        if let Some(team) = info.team {
            owners.insert(team.owner_user_id);
        } else {
            owners.insert(info.owner.id);
        }

        owners
      },
      Err(why) => panic!("Could not access application info: {:?}", why),
    }
  };

  let privileges = GatewayIntents::default()
    .union(GatewayIntents::DIRECT_MESSAGES)
    .union(GatewayIntents::DIRECT_MESSAGE_REACTIONS)
    .union(GatewayIntents::GUILDS)
    .union(GatewayIntents::GUILD_EMOJIS)
    .union(GatewayIntents::GUILD_MEMBERS)
    .union(GatewayIntents::GUILD_MESSAGES)
    .union(GatewayIntents::GUILD_MESSAGE_REACTIONS);

  let mut client = DiscordClient::builder(&token)
    .event_handler(Handler {
      loop_running: AtomicBool::new(false)
    })
    .application_id(app_id)
    .intents(privileges)
    .framework(StandardFramework::new()
    .configure(|c| c
      .owners(owners)
      .prefixes(vec![">", "~"]))
    .help(&MY_HELP)
    .group(&ROLES_GROUP)
    .group(&STATS_GROUP)
    .after(after)
    .on_dispatch_error(dispatch_error))
    .await
    .expect("Error creating client");

  let cat_api_key = var("SAFETY_CAT_KEY")
    .expect("Expected an API key for cats");

  {
    let cat_key = Arc::new(RwLock::new(cat_api_key));

    let mut data = client.data.write().await;
    data.insert::<CatKey>(cat_key);
  }

  {
    let http_arc = Arc::new(http);

    let redis_client = Client::open(redis_url)
      .expect("Should be able to create a redis client");

    let connection = redis_client.get_async_connection().await
      .expect("Should be able to create a redis connection");

    let persistent_connection = redis_client.get_async_connection().await
      .expect("Should be able to create a second redis connection");

    let redis_scheduler = RedisScheduler::new(connection);

    let redis_scheduler_arc = Arc::new(Mutex::new(redis_scheduler));

    let lock = redis_scheduler_arc.clone();

    spawn(async move {
      let mut interval = interval(Duration::from_secs(5));

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
            Err(error) => println!("{:?}", error)
          };
        });


        interval.tick().await;
      };
    });

    let http_clone = client.cache_and_http.http.clone();

    spawn(async move {
      loop {
        let now = Utc::now().with_timezone(&EST5EDT);
        let next_time = now.date().succ().and_hms(0, 0, 30);

        let duration_to_sleep = next_time.signed_duration_since(now)
          .to_std().unwrap();

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
                Err(_) => continue
              };

              if next_time.day() == day && next_time.month() == month {
                let msg = format!("Happy birthday {}! ({} years)!",
                  potential_name[0], next_time.year() - year);

                let result = ChannelId(birthday_announce_channel).say(&http_clone, msg).await;
                println!("{:?}", result);
              }
            }
          },
          Err(error) => {
            println!("Failed to execute query: {:?}", error);
            return;
          }
        };
      }
    });

    {
      let conn_key = Arc::new(Mutex::new(RedisWrapper(persistent_connection)));

      let mut data = client.data.write().await;
      data.insert::<RedisSchedulerKey>(redis_scheduler_arc);
      data.insert::<RedisConnectionKey>(conn_key);
    }
  }

  client.cache_and_http.http.create_global_application_commands(&json!([
    poll_command(),
    roll_command(),
    nya_command(),
    stats_commands()
  ]))
    .await
    .expect("Should be able to create global application commands");

  if let Err(why) = client.start().await {
    println!("An error occurred while running the client: {:?}", why);
  }
}
