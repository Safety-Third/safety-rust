use chrono::{Datelike, Timelike};

use chrono::prelude::{DateTime, NaiveDateTime, TimeZone};
use chrono_tz::{
    America::Anchorage,
    CST6CDT, EST5EDT, MST7MDT, PST8PDT,
    Tz
};

const TIME_FORMATS: [&str; 2] = ["%-m/%-d/%y %-I:%M %p", "%-m/%-d/%y %-H:%M"];

pub fn parse_datetime_tz(date_str: &str) -> Result<DateTime<Tz>, String> {
    let space_idx = match date_str.rfind(" ") {
        Some(index) => index,
        None => {
            return Err(String::from("No spaces in your string. Have you provided a time zone?"));
        }
    };

    let datetime = parse_datetime(&date_str[..space_idx])?;
    let timezone = get_timezone(&date_str[space_idx + 1..])?;

    Ok(timezone.ymd(datetime.year(), datetime.month(), datetime.day())
        .and_hms(datetime.hour(), datetime.minute(), datetime.second()))
}

pub fn change_timezone(date: &DateTime<Tz>, tz: &str) -> Result<DateTime<Tz>, String> {
    let timezone = get_timezone(tz)?;

    Ok(date.with_timezone(&timezone))
}

fn parse_datetime(date_str: &str) -> Result<NaiveDateTime, String> {
    for format in TIME_FORMATS.iter() {
        let date = NaiveDateTime::parse_from_str(date_str, format);

        if date.is_ok() {
            return  Ok(date.unwrap());
        }
    }

    Err(String::from("Could not parse date"))
}

fn get_timezone(tz_string: &str) -> Result<Tz, String> {
    match tz_string {
        "AKST" | "AKDT" => Ok(Anchorage),
        "CST"| "CDT" => Ok(CST6CDT),
        "EDT" => Ok(EST5EDT),
        "MDT" => Ok(MST7MDT),
        "PDT" => Ok(PST8PDT),
        _ => {
            tz_string.parse()
        }
    }
}
