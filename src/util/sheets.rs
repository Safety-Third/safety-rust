use reqwest::{Client, Error};

use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct BatchGet {
  #[serde(rename = "spreadsheetId")]
  pub spreadsheet_id: String,
  #[serde(rename = "valueRanges")]
  pub value_ranges: Vec<ValueRange>
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ValueRange {
  pub range: String,
  #[serde(rename = "majorDimension")]
  pub major_dimension: String,
  pub values: Vec<Vec<String>>
}

pub fn parse_date(date_str: &str) -> Result<(u32, u32, i32), String> {
  let date: Vec<&str> = date_str.split('/').collect();

  if date.len() != 3 {
    return Err(String::from(""));
  }

  let month: u32 = match date[0].parse() {
    Ok(m) => m,
    Err(error) => return Err(error.to_string())
  };

  let day: u32 = match date[1].parse() {
    Ok(d) => d,
    Err(error) => return Err(error.to_string())
  };

  let year: i32 = match date[2].parse() {
    Ok(y) => y,
    Err(error) => return Err(error.to_string())
  };

  Ok((month, day, year))
}

pub async fn query(api_key: &str, sheet_id: &str, ranges: &[&str]) -> Result<BatchGet, Error> {
  let client = Client::new();

  let url = format!("https://sheets.googleapis.com/v4/spreadsheets/{}/values:batchGet", sheet_id);

  let mut query_fields = vec!(("key", api_key));

  for range in ranges.iter() {
    query_fields.push(("ranges", range));
  }

  let result = client.get(&url)
    .query(&query_fields)
    .send()
    .await;

  match result {
    Ok(some) => match some.json::<BatchGet>().await {
      Ok(data) => Ok(data),
      Err(error) => Err(error)
    },
    Err(error) => Err(error)
  }
}