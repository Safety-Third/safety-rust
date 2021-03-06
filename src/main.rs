mod commands;
mod util;

use std::{collections::{HashMap, HashSet}, env::var, sync::Arc, time::Duration};

use chrono::{Datelike, Utc};
use chrono_tz::EST5EDT;
use rand::{distributions::Alphanumeric, Rng,thread_rng};
use redis::{Client, Commands, RedisResult};
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
    channel::{Message, Reaction}, id::{ChannelId, UserId},
    interactions::{
      Interaction,
      InteractionData,
      InteractionResponseType
    }
  },
  prelude::{Context,EventHandler}
};
use tokio::{spawn, sync::Mutex, time };


use commands::{
  events::*,
  impersonate::*,
  poll::*,
  roles::*,
  roll::*,
  stats::*,
  types::{RedisSchedulerKey, RedisConnectionKey, RedisWrapper, Task},
  util::{EMOJI_REGEX, get_guild}
};

use util::{
  scheduler::{Callable, Scheduler as RedisScheduler},
  sheets::{parse_date, query},
};

#[group]
#[commands(impersonate, poll, roll)]
#[description = "General commands"]
struct General;

#[group]
#[commands(cancel, leave, reschedule, schedule, signup)]
#[description = "Create and manage events"]
struct Event;

#[group]
#[commands(add_roles, remove_roles, set_roles)]
#[description = "Commands for managing roles (admin-only)"]
struct Roles;

#[group]
#[commands(
  categories, consent, delete, delete_category, 
  revoke, set_category, stats, uses, view_categories
)]
#[description = "Manage and view emoji stats"]
struct Stats;

struct Handler;

#[async_trait]
impl EventHandler for Handler {
  async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
    if let Some(ref data) = interaction.data {
      if let InteractionData::ApplicationCommand(app_data) = data {
        if let Err(error) = match app_data.name.as_str() {
          "cancel" => interaction_cancel(&ctx, &interaction, &app_data).await,
          "leave" => interaction_leave(&ctx, &interaction, &app_data).await,
          "poll" => interaction_poll(&ctx, &interaction, &app_data).await,
          "reschedule" => interaction_reschedule(&ctx, &interaction, &app_data).await,
          "schedule" => interaction_schedule(&ctx, &interaction, &app_data).await,
          "signup" => interaction_signup(&ctx, &interaction, &app_data).await,
          "roll" => interaction_roll(&ctx, &interaction, &app_data).await,
          _ => Ok(())
        } {
          let _ = interaction.create_interaction_response(&ctx.http, |resp|
            resp.kind(InteractionResponseType::ChannelMessageWithSource)
            .interaction_response_data(|msg|
              msg.content(format!("An error occurred: {}", error)))).await;
        }
      }
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
      match redis_client.0.hgetall(&key) {
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
      let res: RedisResult<String> = redis_client.0.hset_multiple(key, &items);
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
      match redis_client.0.hgetall(&key) {
        Ok(result) => result,
        Err(_) => HashMap::new()
      }
    };

    if data.get("consent") != Some(&1) {
      return;
    }

    let emoji_str = format!("{}", reaction.emoji);

    let new_data = match data.get(&emoji_str) {
      Some(old) => old + 1,
      None => 1
    };


    {
      let mut redis_client = lock.lock().await;
      let res: RedisResult<u64> = redis_client.0.hset(&key, emoji_str, new_data);

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
      match redis_client.0.hgetall(&key) {
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
          redis_client.0.hset(&key, emoji_str, new_data)
        } else {
          redis_client.0.hdel(&key, emoji_str)
        }
      };

      if let Err(error) = res {
        println!("{:?}", error);
      }
    };
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
    println!("{:?}", remaining_args);
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

  let guild_id = var("SAFETY_GUILD_ID")
    .expect("expected a guild id for guild commands")
    .parse::<u64>()
    .expect("Expected the id to be a number");

  let birthday_sheet_id = var("SAFETY_GOOGLE_DOCS_LINK")
    .expect("Expected Safety Google Docs link");

  let birthday_vector = vec!("A2:A51", "J2:J51");

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
  
  let mut client = DiscordClient::builder(&token)
    .event_handler(Handler)
    .application_id(app_id)
    .intents(GatewayIntents::all())
    .framework(StandardFramework::new()
    .configure(|c| c
      .owners(owners)
      .prefixes(vec![">", "~"]))
    .help(&MY_HELP)
    .group(&EVENT_GROUP)
    .group(&GENERAL_GROUP)
    .group(&ROLES_GROUP)
    .group(&STATS_GROUP)
    .after(after)
    .on_dispatch_error(dispatch_error))
    .await
    .expect("Error creating client");

  {
    let http_arc = Arc::new(http);

    let generator = || -> String {
      thread_rng()
        .sample_iter(&Alphanumeric)
        .take(6)
        .collect()
    };

    let redis_client = Client::open(redis_url)
      .expect("Should be able to create a redis client");

    let connection = redis_client.get_connection()
      .expect("Should be able to create a redis connection");

    let persistent_connection = redis_client.get_connection()
      .expect("Should be able to create a second redis connection");

    let redis_scheduler: RedisScheduler<Task, Arc<Http>> = 
      RedisScheduler::new(connection, Some(Box::new(generator)), None, None);

    let redis_scheduler_arc = Arc::new(Mutex::new(redis_scheduler));

    let lock = redis_scheduler_arc.clone();

    
    spawn(async move {
      let mut interval = time::interval(Duration::from_secs(5));

      loop {
        let jobs = {
          let now = Utc::now().timestamp();
          let mut task_scheduler = lock.lock().await;
          task_scheduler.get_and_clear_ready_jobs(now)
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

        time::sleep(duration_to_sleep).await;

        if let Ok(sheet) = query(&api_key, &birthday_sheet_id, &birthday_vector).await {
          if sheet.value_ranges.len() != 2 {
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
        }
      }
    });

    {
      let conn_key = Arc::new(Mutex::new(RedisWrapper(persistent_connection)));

      let mut data = client.data.write().await;
      data.insert::<RedisSchedulerKey>(redis_scheduler_arc);
      data.insert::<RedisConnectionKey>(conn_key);
    }
  }

  let res = client.cache_and_http.http.create_guild_application_commands(guild_id, &json!([
    cancel_command(),
    leave_command(),
    reschedule_command(),
    roll_command(),
    schedule_command(),
    signup_command(),
    poll_command()
  ])).await;

  println!("{:?}", res);

  if let Err(why) = client.start().await {
    println!("An error occurred while running the client: {:?}", why);
  }
}
