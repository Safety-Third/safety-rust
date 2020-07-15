mod datetime_parse;

use datetime_parse::*;

fn main() {
    println!("{:?}", change_timezone(&parse_datetime_tz("1/1/20 7:00 PM EDT").unwrap(), "CST"));
}
