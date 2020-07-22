mod commands;
mod util;

use chrono::Utc;
use chrono_tz::EST5EDT;
use clokwerk::{Scheduler, ScheduleHandle, TimeUnits};
use lazy_static::lazy_static;
use parking_lot::Mutex;
use redis::{Commands, Client};
use regex::Regex;
use serenity::{
  client::{Client as DiscordClient},
  framework::standard::{
    Args, CommandGroup, CommandResult, HelpOptions, 
    help_commands, StandardFramework,
    macros::{group, help}
  },
  http::Http,
  model::{
    channel::{Channel, Message}, gateway::Ready, id::UserId, user::OnlineStatus
  },
  prelude::{Context,EventHandler}
};
use threadpool::ThreadPool;

use std::{collections::{HashMap, HashSet}, sync::Arc, time::Duration};

use commands::{
  events::*, impersonate::*, poll::*, roles::*, roll::*,
  types::{ClokwerkSchedulerKey, RedisSchedulerKey, RedisConnectionKey, Task}
};

use util::scheduler::{Callable, Scheduler as RedisScheduler};

#[group]
#[commands(cancel, impersonate, leave, poll, roll, schedule, signup)]
#[description = "General commands"]
struct General;

#[group]
#[commands(add_roles, remove_roles, set_roles)]
#[description = "Commands for managing roles (admin-only)"]
#[prefixes("roles", "ro")]
struct Roles;

use std::env;

const THREAD_COUNT: usize = 10;

lazy_static!{
  static ref EMOJI_REGEX: Regex = Regex::new(r"(?x)
    (
      \p{Emoji_Modifier_Base}|
      \p{Emoji_Modifier}|
      \p{Emoji_Component}|
      <a?:[a-zA-Z0-9_]+:[0-9]+>
    )").unwrap();
}

struct Handler;

impl EventHandler for Handler {
  fn ready(&self, ctx: Context, _: Ready) {


  }

  /*fn message(&self, ctx: Context, msg: Message) {
    if msg.is_own(&ctx.cache) {
        return;
    }

    let channel = match msg.channel(&ctx.cache) {
      Some(c) => c,
      None => {
        return;
      }
    };

    let guild_channel = match channel {
      Channel::Guild(c) => c,
      _ => { return; }
    };

    let guild = match get_guild(&ctx, &msg) {
      Ok(g) => g,
      Err(_) => { return; }
    };

    let key = {
      let guild = guild.read();
      format!("{}:{}", msg.author.id.0, guild.id.0)
    };

    let lock = {
      let mut context = ctx.data.write();
      context.get_mut::<RedisConnectionKey>()
        .expect("Expected redis instance")
        .clone()
    };

    let mut data: HashMap<String, u64> = { 
      let mut redis_client = lock.lock();
      match redis_client.hgetall(&key) {
        Ok(result) => result,
        Err(_) => HashMap::new()
      }
    };

    if data.get("consent") != Some(&1) {
      return;
    }

    for mat in EMOJI_REGEX.find_iter(&msg.content) {
      println!("{:?}", &msg.content[mat.start()..mat.end()]);
    }
  }*/
}

#[help]
#[individual_command_tip = "Henlo, welcome to Bot v2.\n\
For help on a specific command, just pass that name in."]
#[command_not_found_text = "Could not find: `{}`."]
#[max_levenshtein_distance(3)]
#[lacking_permissions = "Hide"]
#[lacking_role = "Nothing"]
#[wrong_channel = "Strike"]
fn my_help(
  context: &mut Context, msg: &Message, args: Args,
  help_options: &'static HelpOptions, groups: &[&'static CommandGroup],
  owners: HashSet<UserId>
) -> CommandResult {
  help_commands::with_embeds(context, msg, args, help_options, groups, owners)
}

fn main() {
  // Login with a bot token from the environment
  let mut client = DiscordClient::new(&env::var("RUST_BOT").expect("token"), Handler)
      .expect("Error creating client");

  let owners = match client.cache_and_http.http.get_current_application_info() {
    Ok(info) => {
      let mut set = HashSet::new();
      set.insert(info.owner.id);

      set
    },
    Err(why) => panic!("Couldn't get application info: {:?}", why),
  };

  let thread_pool = ThreadPool::new(THREAD_COUNT);

  {
    let redis_client = Client::open("redis://127.0.0.1/")
      .expect("Should be able to create a redis client");

    let connection = redis_client.get_connection()
      .expect("Should be able to create a redis connection");

    let persistent_connection = redis_client.get_connection()
      .expect("Should be able to create a second redis connection");

    let mut scheduler = Scheduler::with_tz(EST5EDT);

    let redis_scheduler: RedisScheduler<Task, Arc<Http>> = 
      RedisScheduler::new(connection, None, None);

    let redis_scheduler_arc = Arc::new(Mutex::new(redis_scheduler));

    let lock = redis_scheduler_arc.clone();
    let pool = thread_pool.clone();
    let http = client.cache_and_http.http.clone();

    scheduler.every(5.seconds()).run(move|| {
      let jobs = {
        let now = Utc::now().timestamp();
        let mut task_scheduler = lock.lock();
        task_scheduler.get_and_clear_ready_jobs(now)
      };

      match jobs {
        Ok(tasks) => {
          for job in tasks.iter() {
            let http_clone = http.clone();
            let clone = job.clone();
            pool.execute(move || {
              clone.call(&http_clone);
            });
          }
        }
        Err(error) => println!("{:?}", error)
      };
    });

    let handler = scheduler.watch_thread(Duration::from_millis(500));

    {
      let mut data = client.data.write();
      data.insert::<RedisSchedulerKey>(redis_scheduler_arc);
      data.insert::<RedisConnectionKey>(Arc::new(Mutex::new(persistent_connection)));
      data.insert::<ClokwerkSchedulerKey>(Arc::new(handler));
    }
  }

  client.threadpool = thread_pool;
  
  client.with_framework(StandardFramework::new()
    .configure(|c| c
      .owners(owners)
      .prefix(">"))
    .help(&MY_HELP)
    .group(&GENERAL_GROUP)
    .group(&ROLES_GROUP)
    .after(|ctx, msg, _, error| {
      if error.is_err() {
        let _ = msg.channel_id.say(&ctx.http, 
          &format!("Error in {:?}:\n{}", msg.content, error.unwrap_err().0));
      }
    })
    .on_dispatch_error(|ctx, msg, error| {
        let _ = msg.channel_id.say(&ctx.http, 
          &format!("Error in {:?}:\n{:?}", msg.content, error));
    }));

  if let Err(why) = client.start() {
    println!("An error occurred while running the client: {:?}", why);
  }
}
