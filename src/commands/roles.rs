use serenity::prelude::*;
use serenity::model::prelude::*;
use serenity::framework::standard::{
  Args, CommandResult, macros::command,
};

use super::util::*;

#[command("addRoles")]
#[aliases("add_roles", "addR", "add_r")]
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
pub async fn add_roles(ctx: &Context, msg: &Message, args: Args) -> CommandResult {
  change_roles(ctx, msg, args, |input_roles, member_roles| {
    input_roles.extend(member_roles);

    input_roles.sort();
    input_roles.dedup();
  }).await
}

#[command("removeRoles")]
#[aliases("remove_roles", "remR", "rem_r")]
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
pub async fn remove_roles(ctx: &Context, msg: &Message, args: Args) -> CommandResult {
  change_roles(ctx, msg, args, |input_roles, member_roles| {
    *input_roles = member_roles
      .iter()
      .filter(|role| !input_roles.contains(role))
      .cloned()
      .collect();
  }).await
}

#[command("setRoles")]
#[aliases("set_roles", "setR", "set_r")]
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
pub async fn set_roles(ctx: &Context, msg: &Message, args: Args) -> CommandResult {
  change_roles(ctx, msg, args, |_, _| {}).await
}

/// This function is responsible for getting the roles for a current user,
/// and calling a generic function, `func`, to change the user's roles 
/// 
/// # Arguments
/// - `ctx`:  the context of this current message
/// - `msg`:  the original message for this request
/// - `args`: the arguments for this function
/// - `func`: a handler that takes as its input a vector of roles generated from `args`,
/// and a vector of roles that the user currently has. The former vector is mutated
/// 
/// # Returns
/// - `Err`: if a user could not be found, one or more roles could not be found,
/// or updating the user's role(s) failed
/// - `Ok`: otherwise
async fn change_roles<F>(ctx: &Context, msg: &Message, mut args: Args, mut func: F) -> CommandResult 
  where F: FnMut(&mut Vec<RoleId>, &Vec<RoleId>) {

  let guild = get_guild(ctx, msg).await?;
  let args_user = args.parse::<UserId>();

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
      Err(error) => return command_err!(error.to_string())
    }
  };

  func(&mut roles, &member.roles);

  match guild.edit_member(&ctx, member_id, |m| {
    m.roles(&roles)
  }).await {
    Ok(_) => {
      let roles: Vec<String> = roles
        .into_iter()
        .map(|role| role.mention().to_string())
        .collect();

      let _ = msg.channel_id.say(&ctx.http, 
        format!("New roles for {}: {:?}",
          member.mention(), roles)).await;

      Ok(())
    },
    Err(error) => command_err!(error.to_string())
  }
}
