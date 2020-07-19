use serenity::prelude::*;
use serenity::model::prelude::*;
use serenity::framework::standard::{
  Args, CommandError, CommandResult,
  macros::command,
};

use super::util::*;

#[command("add")]
#[example("person role-a \"role b\"")]
#[min_args(2)]
#[required_permissions("ADMINISTRATOR")]
#[usage("user roles")]
/// Add one or more roles to a user
/// 
/// Accepts users and roles in the form of their id, name (in quotes as needed), or mention:
/// 
/// * Users: `@user`, `"user"`, or `000000000000000000`
/// * Roles: `@role`, `"role"`, or `000000000000000000`
/// * Roles list is separated by spaces: `role-a @role-b "role c"`
pub fn add_roles(ctx: &mut Context, msg: &Message, args: Args) -> CommandResult {
  change_roles(ctx, msg, args, |input_roles, member_roles| {
    input_roles.extend(member_roles);

    input_roles.sort();
    input_roles.dedup();
  })
}

#[command("remove")]
#[description = "Remove one or more roles to a user"]
#[example("person role-a \"role b\"")]
#[min_args(2)]
#[required_permissions("ADMINISTRATOR")]
#[usage("user roles")]
/// Removes one or more roles to a user
/// 
/// Accepts users and roles in the form of their id, name (in quotes as needed), or mention:
/// 
/// * Users: `@user`, `"user"`, or `000000000000000000`
/// * Roles: `@role`, `"role"`, or `000000000000000000`
/// * Roles list is separated by spaces: `role-a @role-b "role c"`
pub fn remove_roles(ctx: &mut Context, msg: &Message, args: Args) -> CommandResult {
  change_roles(ctx, msg, args, |input_roles, member_roles| {
    *input_roles = member_roles
      .iter()
      .filter(|role| !input_roles.contains(role))
      .cloned()
      .collect();
  })
}

#[command("set")]
#[description = "Set one or more roles for a user"]
#[example("person role-a \"role b\"")]
#[min_args(1)]
#[required_permissions("ADMINISTRATOR")]
#[usage("user roles")]
/// Sets the user's current roles to the input list
/// 
/// Accepts users and roles in the form of their id, name (in quotes as needed), or mention:
///
/// * Users: `@user`, `"user"`, or `000000000000000000`
/// * Roles: `@role`, `"role"`, or `000000000000000000`
/// * Roles list is separated by spaces: `role-a @role-b "role c"`
/// 
/// You can provide no roles to remove all roles (aside from @everyone)
pub fn set_roles(ctx: &mut Context, msg: &Message, args: Args) -> CommandResult {
  change_roles(ctx, msg, args, |_, _| {})
}

fn change_roles<F>(ctx: &mut Context, msg: &Message, mut args: Args, mut func: F) -> CommandResult 
  where F: FnMut(&mut Vec<RoleId>, &Vec<RoleId>) {

  let guild = get_guild(ctx, msg)?;
  let args_user = args.parse::<UserId>();

  let guild = guild.read();

  let member_id: UserId = if args_user.is_ok() {
    args.single::<UserId>().unwrap()
  } else {
    let member_name = args.single_quoted::<String>()?;
    get_user_from_string(&guild, &member_name)?
  };

  let member = match guild.members.get(&member_id) {
    Some(user) => user,
    None => return error!("user with id", member_id.0)
  };

  let mut roles: Vec<RoleId> = Vec::new();

  for role in args.iter::<String>() {
    match role {
      Ok(role_string) => {
        match get_role_from_string(&guild, &role_string) {
          Ok(role_id) => roles.push(role_id),
          Err(error) => return Err(error)
        }
      },
      Err(error) => return Err(CommandError(error.to_string()))
    }
  };

  func(&mut roles, &member.roles);

  match guild.edit_member(&ctx, member_id, |m| {
    m.roles(&roles)
  }) {
    Ok(_) => {
      let roles: Vec<String> = roles
        .into_iter()
        .map(|role| role.mention())
        .collect();

      let _ = msg.channel_id.say(&ctx.http, 
        format!("New roles for {}: {:?}",
          member.mention(), roles));

      Ok(())
    },
    Err(error) => Err(CommandError(error.to_string()))
  }
}