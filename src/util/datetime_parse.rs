use chrono::{Datelike, Timelike};

use chrono::prelude::{DateTime, NaiveDateTime, TimeZone};
use chrono_tz::{
    America::Anchorage,
    CST6CDT, EST5EDT, MST7MDT, PST8PDT,
    Tz
};

const TIME_FORMATS: [&str; 4] = [
    "%-m/%-d/%y %-I:%M %p", "%-m/%-d/%y %-H:%M",
    "%-m/%-d/%y %-I:%M:%S %p", "%-m/%-d/%y %-H:%M:%S"
];

pub fn parse_datetime_tz(date_str: &str) -> Result<DateTime<Tz>, String> {
    let space_idx = match date_str.rfind(' ') {
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

fn parse_datetime(date_str: &str) -> Result<NaiveDateTime, String> {
    for format in TIME_FORMATS.iter() {
        let date = NaiveDateTime::parse_from_str(date_str, format);

        if let Ok(result) = date {
            return Ok(result);
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
