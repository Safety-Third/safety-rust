mod commands;

use serenity::{
    client::Client,
    framework::standard::{
        Args, CommandGroup, CommandResult, HelpOptions, 
        help_commands, StandardFramework,
        macros::{group, help}
    },
    model::{
        channel::{Message}, id::UserId
    },
    prelude::{EventHandler, Context}
};

use std::{collections::HashSet};

use commands::{
    impersonate::*, roles::*, roll::*
};

#[group]
#[commands(impersonate, roll)]
#[description = "General commands"]
struct General;

#[group]
#[commands(add_roles, remove_roles, set_roles)]
#[description = "Commands for managing roles (admin-only)"]
#[prefixes("roles", "ro")]
struct Roles;

use std::env;

struct Handler;

impl EventHandler for Handler {}

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
    let mut client = Client::new(&env::var("RUST_BOT").expect("token"), Handler)
        .expect("Error creating client");

    let owners = match client.cache_and_http.http.get_current_application_info() {
        Ok(info) => {
            let mut set = HashSet::new();
            set.insert(info.owner.id);

            set
        },
        Err(why) => panic!("Couldn't get application info: {:?}", why),
    };    
    
    client.with_framework(StandardFramework::new()
        .configure(|c| c
            .owners(owners)
            .prefix("~")) // set the bot's prefix to "~"
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

    
    // start listening for events by starting a single shard
    if let Err(why) = client.start() {
        println!("An error occurred while running the client: {:?}", why);
    }
}
