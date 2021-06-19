use lazy_static::lazy_static;
use rand::{Rng,thread_rng};
use regex::Regex;

use serenity::prelude::*;
use serenity::model::prelude::*;
use serenity::framework::standard::{
  Args, CommandResult, macros::command,
};

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use super::util::handle_command_err;

#[command]
#[min_args(1)]
#[usage("die_list")]
#[example("1d20 2d4 3d125129")]
#[example("3d8+5 2d6-8")]
#[example("8d10dl2")]
#[example("8d10dh3")]
#[example("8d10dldh")]
#[example("10d20+2dl2dh2")]
/// Rolls one or more dice. Dice rolls should be in this general form:
/// "int"d"int"
/// >roll 1d20
/// 
/// You can also add modifiers to the roll:
/// +/-"int"
/// >roll 3d8+5 
/// 
/// And you can drop the n highest/lowest rolls:
/// dl"int"dh"int" (you can omit "int" to drop 1)
/// >roll 8d10dl2: drop 2 lowest
/// >roll 8d10dh: drop highest
/// >roll 8d10dldh: drop lowest and highest
/// 
/// Put together, we have:
/// "int"d"int"+/-"int"dl"int"dh"int" 
/// >roll 10d20+2dl2dh2: 10 d 20s, +2, drop 2 lowest and highest
pub async fn roll(ctx: &Context, msg: &Message, args: Args) -> CommandResult {
  let mut total_string = String::from(">>> ");

  let mut total_sum: i32 = 0;

  for die_roll in args.raw_quoted() {
    match handle_roll(&die_roll) {
      Ok((count, message)) => {
        total_sum += count;
        total_string += &message;
      },
      Err(error) => return handle_command_err(ctx, msg, &error).await
    };
  }

  let _ = msg.channel_id.say(&ctx.http, 
    format!("{}, you rolled a total of **{}**\n{}\n", 
      msg.author.mention(), total_sum, total_string)).await;

  Ok(())
}

fn handle_roll(roll_str: &str) -> Result<(i32, String), String> {
  lazy_static! {
    static ref RE: Regex = Regex::new(r"(?x)
      (?P<count>\d+)  # number of times to roll this die
      d(?P<size>\d+)  # number of sides (nonzero)
      ((?P<addition>[+-]\d+))?  # modifier for the overall roll
      (dl(?P<low>\d*))?         # how many high rolls to drop
      (dh(?P<high>\d*))?        # how many lows to drop"
    ).unwrap();
  }
  
  let caps = match RE.captures(roll_str) {
    Some(captures) => captures,
    None =>  return error_hash(roll_str, "Not a valid die roll")
  };

  let size = match caps.name("size").unwrap().as_str().parse::<u32>() {
    Ok(int) => int,
    Err(_) => return error_hash(roll_str, "Must provide a valid, nonnegative size")
  };

  if size == 0 {
    return error_hash(roll_str, "I will not roll a d0");
  }

  let count = match caps.name("count").unwrap().as_str().parse::<u32>() {
    Ok(int) => int,
    Err(_) => return error_hash(roll_str, "Must provide a valid, nonnegative number of dice")
  };

  if count == 0 {
    return error_hash(roll_str, "I *can* roll zero dice, but am morally obligated not to");
  }

  let addition = match caps.name("addition") {
    Some(number) => {
      match number.as_str().parse::<i32>() {
        Ok(int) => int,
        Err(_) => return error_hash(roll_str, "Modifier must be a number")
      }
    },
    None => 0
  };

  let low = match caps.name("low") {
    Some(value) => {
      let string_val = value.as_str();
      if string_val == "" {
        1
      } else {
        match string_val.parse::<u32>() {
          Ok(int) => int,
          Err(_) => return error_hash(roll_str, "Low drop must be a number")
        }
      }
    },
    None => 0
  };

  let high = match caps.name("high") {
    Some(value) => {
      let string_val = value.as_str();
      if string_val == "" {
        1
      } else {
        match string_val.parse::<u32>() {
          Ok(int) => int,
          Err(_) => return error_hash(roll_str, "High drop must be a number")
        }
      }
    },
    None => 0
  };
  
  if low + high >= count {
    return error_hash(roll_str, &format!(
      "You want to drop {} dice but are only rolling {}. What are you even doing?",
      low + high, count));
  }

  let mut rolls: Vec<i32> = Vec::new();
  let mut rng = thread_rng();

  for _ in 0..count {
    let result = rng.gen_range(1, size + 1) as i32;
    rolls.push(result);
  }

  rolls.sort();

  let meaningful_rolls = if low > 0 || high > 0 {
    &rolls[(low as usize)..(rolls.len() - (high as usize))]
  } else {
    &rolls[..]
  };

  let sum: i32 = meaningful_rolls.iter().sum::<i32>();
  let avg = (sum as f64) / (meaningful_rolls.len() as f64);

  let mut result_str = format!("{}d{}", count, size);

  if addition != 0 {
    result_str += &format!(" + {}", addition);
  }

  if low > 0 {    
    result_str += &format!(", drop {} low", low);
  }

  if high > 0 {    
    result_str += &format!(", drop {} high", high);
  }

  let full_rolls = pretty_vec(meaningful_rolls);

  result_str += &format!(" = **{}**\n{} (avg {})\n", sum + addition, full_rolls, avg);

  if low > 0 {
    let low_set = &rolls[0..(low as usize)];
    let low_sum = low_set.iter().sum::<i32>();
    let low_avg = (low_sum as f64) / (low_set.len() as f64);
    result_str += &format!("Dropped low: {} (avg {})\n", pretty_vec(low_set), low_avg);
  }

  if high > 0 {
    let high_set = &rolls[rolls.len() - (high as usize)..];
    let high_sum = high_set.iter().sum::<i32>();
    let high_avg = (high_sum as f64) / (high_set.len() as f64);
    result_str += &format!("Dropped low: {} (avg {})\n", pretty_vec(high_set), high_avg);
  }

  Ok((sum, result_str))
}

/// Hashes and rolls an invalid die roll and returns the error message.
/// This will always return an error
/// # Arguments
/// - die:    a string representing an invalid die roll, to be hashed
/// - error:  the error message corresponding to the roll
fn error_hash(die: &str, error: &str) -> Result<(i32, String), String> {
  let mut s = DefaultHasher::new();
  die.hash(&mut s);
  
  let hashed_val = (s.finish() % 1000) as u32;

  let rng = thread_rng().gen_range(1, hashed_val + 1);

  Err(
    format!("**{}**.\nBut here's a guess for {} = 1d{}: **{}**", error, die, hashed_val, rng))
}

/// Creates a String representation of `numbers`, separated by commas
/// # Arguments
/// - `numbers` - a vector of numbers
fn pretty_vec(numbers: &[i32]) -> String {
  let string_list: Vec<String> = numbers
    .iter()
    .map(|number| number.to_string())
    .collect();

  string_list.join(", ")
}
