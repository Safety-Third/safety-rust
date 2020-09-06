mod commands;
mod util;

use chrono::{Datelike, Utc};
use chrono_tz::EST5EDT;
use clokwerk::{Scheduler, TimeUnits};
use parking_lot::Mutex;
use rand::{
  distributions::Alphanumeric, Rng,thread_rng
};
use redis::{Client, Commands, RedisResult};
use serenity::{
  client::{Client as DiscordClient},
  framework::standard::{
    Args, Delimiter, CommandGroup, CommandResult, HelpOptions, 
    help_commands, StandardFramework,
    macros::{group, help}
  },
  http::Http,
  model::{
    channel::{Message, Reaction}, id::{ChannelId, UserId},
  },
  prelude::{Context,EventHandler}
};
use tokio::runtime::Runtime;

use std::{collections::{HashMap, HashSet}, env::var, sync::Arc, time::Duration};

use commands::{
  events::*, impersonate::*, poll::*, roles::*, roll::*, stats::*,
  types::{ClokwerkSchedulerKey, RedisSchedulerKey, RedisConnectionKey, RedisWrapper, Task},
  util::{EMOJI_REGEX, get_guild}
};

use util::{
  scheduler::{Callable, Scheduler as RedisScheduler},
  sheets::{parse_date, query}
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

const THREAD_COUNT: usize = 5;

struct Handler;

impl EventHandler for Handler {
  fn message(&self, ctx: Context, msg: Message) {
    if msg.author.bot {
      return;
    }

    let id = match msg.guild_id {
      Some(i) => i.0,
      None => {
        match get_guild(&ctx, &msg) {
          Ok(guild) => guild.read().id.0,
          Err(_) => return
        }
      }
    };

    let key = format!("{}:{}", msg.author.id.0, id);

    let lock = {
      let mut context = ctx.data.write();
      context.get_mut::<RedisConnectionKey>()
        .expect("Expected redis instance")
        .clone()
    };

    let mut data: HashMap<String, u64> = { 
      let mut redis_client = lock.lock();
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
      let mut redis_client = lock.lock();
      let res: RedisResult<String> = redis_client.0.hset_multiple(key, &items);
      if let Err(error) = res {
        println!("{:?}", error);
      }
    }
  }
  
  fn reaction_add(&self, ctx: Context, reaction: Reaction) {
    let guild_id = match reaction.guild_id {
      Some(id) => id,
      None => return
    };

    let user = match reaction.user(&ctx.http) {
      Ok(u) => u,
      Err(_) => return
    };

    if user.bot {
      return;
    }


    let key = format!("{}:{}", reaction.user_id.0, guild_id);

    let lock = {
      let mut context = ctx.data.write();
      context.get_mut::<RedisConnectionKey>()
        .expect("Expected redis instance")
        .clone()
    };

    let data: HashMap<String, u64> = { 
      let mut redis_client = lock.lock();
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
      let mut redis_client = lock.lock();
      let res: RedisResult<u64> = redis_client.0.hset(&key, emoji_str, new_data);

      if let Err(error) = res {
        println!("{:?}", error);
      }
    };
  }

  fn reaction_remove(&self, ctx: Context, reaction: Reaction) {
    let guild_id = match reaction.guild_id {
      Some(id) => id,
      None => return
    };

    let user = match reaction.user(&ctx.http) {
      Ok(u) => u,
      Err(_) => return
    };

    if user.bot {
      return;
    }

    let key = format!("{}:{}", reaction.user_id.0, guild_id);

    let lock = {
      let mut context = ctx.data.write();
      context.get_mut::<RedisConnectionKey>()
        .expect("Expected redis instance")
        .clone()
    };

    let data: HashMap<String, u64> = { 
      let mut redis_client = lock.lock();
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
      let mut redis_client = lock.lock();

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
#[individual_command_tip = "Henlo, welcome to Bot v2.\n\
For help on a specific command, just pass that name in.
To see this in embed form, pass `embed` as your first option (e.g. `help embed other_stuff`)"]
#[command_not_found_text = "Could not find: `{}`."]
#[max_levenshtein_distance(3)]
#[lacking_permissions = "Hide"]
#[lacking_role = "Nothing"]
#[wrong_channel = "Strike"]
fn my_help(
  context: &mut Context, msg: &Message, mut args: Args,
  help_options: &'static HelpOptions, groups: &[&'static CommandGroup],
  owners: HashSet<UserId>
) -> CommandResult {
  let embed = args.current() == Some("embed");

  if embed {
    args.advance();
    let remaining_args = Args::new(args.remains().unwrap_or(""), &[Delimiter::Single(' ')]);
    help_commands::with_embeds(context, msg, remaining_args, help_options, groups, owners)
  } else {
    help_commands::plain(context, msg, args, help_options, groups, owners)
  }
}

fn main() {
  let api_key = var("GOOGLE_API_KEY")
    .expect("Expected Google API key");

  let birthday_announce_channel = var("SAFETY_ANNOUNCEMENT_CHANNEL")
    .expect("Expected birthday announcement channel")
    .parse::<u64>()
    .expect("Expected channel to be a number");

  let birthday_sheet_id = var("SAFETY_GOOGLE_DOCS_LINK")
    .expect("Expected Safety Google Docs link");

  let redis_url = var("SAFETY_REDIS_URL")
    .unwrap_or_else(|_| String::from("redis://127.0.0.1"));

  let mut client = DiscordClient::new(&var("RUST_BOT").expect("token"), Handler)
      .expect("Error creating client");
      
  let owners = match client.cache_and_http.http.get_current_application_info() {
    Ok(info) => {
      let mut set = HashSet::new();
      set.insert(info.owner.id);

      set
    },
    Err(why) => panic!("Couldn't get application info: {:?}", why),
  };

  let runtime = Runtime::new()
    .expect("Expected tokio runtime");

  {
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

    let mut scheduler = Scheduler::with_tz(EST5EDT);

    let redis_scheduler: RedisScheduler<Task, Arc<Http>> = 
      RedisScheduler::new(connection, Some(Box::new(generator)), None, None);

    let redis_scheduler_arc = Arc::new(Mutex::new(redis_scheduler));

    let lock = redis_scheduler_arc.clone();
    let pool = client.threadpool.clone();
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

    let handle = runtime.handle().clone();
    let birthday_vector = vec!("A2:A51", "J2:J51");
    let http = client.cache_and_http.http.clone();

    scheduler.every(1.day()).at("00:00:30").run(move || {
      let future = query(&api_key, &birthday_sheet_id, &birthday_vector);
      let now = Utc::now().with_timezone(&EST5EDT);

      if let Ok(sheet) = handle.block_on(future) {
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

          if now.day() == day && now.month() == month {
            let msg = format!("Happy birthday {}! ({} years)!",
              potential_name[0], now.year() - year);

            let result = ChannelId(birthday_announce_channel).say(&http, msg);
            println!("{:?}", result);
          }
        }
      }
    });

    let handler = scheduler.watch_thread(Duration::from_millis(500));

    {
      let conn_key = Arc::new(Mutex::new(RedisWrapper(persistent_connection)));

      let mut data = client.data.write();
      data.insert::<RedisSchedulerKey>(redis_scheduler_arc);
      data.insert::<RedisConnectionKey>(conn_key);
      data.insert::<ClokwerkSchedulerKey>(Arc::new(handler));
    }
  }

  client.threadpool.set_num_threads(THREAD_COUNT);
  
  client.with_framework(StandardFramework::new()
    .configure(|c| c
      .owners(owners)
      .prefixes(vec![">", "~"]))
    .help(&MY_HELP)
    .group(&EVENT_GROUP)
    .group(&GENERAL_GROUP)
    .group(&ROLES_GROUP)
    .group(&STATS_GROUP)
    .after(|ctx, msg, _, error| {
      if error.is_err() && !msg.author.bot {
        let _ = msg.channel_id.say(&ctx.http, 
          &format!("Error in {:?}:\n{}", msg.content, error.unwrap_err().0));
      }
    })
    .on_dispatch_error(|ctx, msg, error| {
      if !msg.author.bot {
        let _ = msg.channel_id.say(&ctx.http, 
          &format!("Error in {:?}:\n{:?}", msg.content, error));
      }
    }));

  if let Err(why) = client.start() {
    println!("An error occurred while running the client: {:?}", why);
  }
}
