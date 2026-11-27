//! Shopfloor: an RFQ-to-quote command Spore over `soil:records`.
//!
//! No bespoke auth, tenancy, or network. The host selects the records book and
//! therefore the Desktop/tenant; this module only reads and writes the
//! `shopfloor` collection it is granted.

wit_bindgen::generate!({
    path: "wit",
    world: "command",
    generate_all,
});

use serde_json::{Value, json};
use soil::records::records::{self, Entry, Key};

const COLLECTION: &str = "shopfloor";
const RFQ_SCHEMA: &str = "fpl.shopfloor/rfq@1";
const QUOTE_SCHEMA: &str = "fpl.shopfloor/quote@1";

struct Component;
export!(Component);

impl exports::wasi::cli::run::Guest for Component {
    fn run() -> Result<(), ()> {
        match main() {
            Ok(()) => Ok(()),
            Err(error) => {
                eprintln!("shopfloor: {error}");
                Err(())
            }
        }
    }
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let action = args.first().map(String::as_str).unwrap_or("list");
    match action {
        "list" => {
            let mut cursor = None;
            let mut rows = Vec::new();
            loop {
                let page = records::scan(COLLECTION, cursor.as_deref(), 256)
                    .map_err(|error| format!("scan: {error:?}"))?;
                for entry in page.entries {
                    rows.push(render(&entry));
                }
                match page.next {
                    Some(next) => cursor = Some(next),
                    None => break,
                }
            }
            println!("{}", serde_json::to_string_pretty(&rows).map_err(|e| e.to_string())?);
            Ok(())
        }
        "rfq" if args.get(1).map(String::as_str) == Some("create") => {
            let title = args.get(2).ok_or("usage: rfq create TITLE CUSTOMER [ID]")?;
            let customer = args.get(3).ok_or("usage: rfq create TITLE CUSTOMER [ID]")?;
            let id = args.get(4).cloned().unwrap_or_else(|| format!("rfq-{}", slug(title)));
            let body = json!({"kind":"rfq","title":title,"customer":customer,"status":"open"});
            put(&id, RFQ_SCHEMA, body)?;
            println!(r#"{{"id":"{id}","kind":"rfq","status":"open"}}"#);
            Ok(())
        }
        "quote" if args.get(1).map(String::as_str) == Some("create") => {
            let rfq = args.get(2).ok_or("usage: quote create RFQ AMOUNT LEAD_DAYS [ID]")?;
            let amount: f64 = args
                .get(3)
                .ok_or("usage: quote create RFQ AMOUNT LEAD_DAYS [ID]")?
                .parse()
                .map_err(|_| "amount must be a number".to_string())?;
            let lead: u32 = args
                .get(4)
                .ok_or("usage: quote create RFQ AMOUNT LEAD_DAYS [ID]")?
                .parse()
                .map_err(|_| "lead days must be an integer".to_string())?;
            if records::get(&Key { collection: COLLECTION.into(), id: rfq.clone() })
                .map_err(|error| format!("rfq lookup: {error:?}"))?
                .deleted
            {
                return Err(format!("rfq {rfq} is deleted"));
            }
            let id = args
                .get(5)
                .cloned()
                .unwrap_or_else(|| format!("quote-{rfq}-{}", (amount * 100.0) as i64));
            let body = json!({"kind":"quote","rfq":rfq,"amount":amount,"lead_days":lead,"status":"draft"});
            put(&id, QUOTE_SCHEMA, body)?;
            println!(r#"{{"id":"{id}","kind":"quote","rfq":"{rfq}","status":"draft"}}"#);
            Ok(())
        }
        "help" => {
            println!("shopfloor list | rfq create TITLE CUSTOMER [ID] | quote create RFQ AMOUNT LEAD_DAYS [ID]");
            Ok(())
        }
        other => Err(format!("unknown command {other:?}; use help")),
    }
}

fn put(id: &str, schema: &str, body: Value) -> Result<(), String> {
    let body = serde_json::to_vec(&body).map_err(|e| e.to_string())?;
    records::upsert(&Entry {
        key: Key { collection: COLLECTION.into(), id: id.into() },
        schema: schema.into(),
        revision: 0,
        updated_at_ms: 0,
        origin: String::new(),
        deleted: false,
        body,
    })
    .map_err(|error| format!("upsert: {error:?}"))?;
    Ok(())
}

fn render(entry: &Entry) -> Value {
    let body = serde_json::from_slice::<Value>(&entry.body).unwrap_or(Value::Null);
    json!({
        "id": entry.key.id,
        "schema": entry.schema,
        "revision": entry.revision,
        "origin": entry.origin,
        "deleted": entry.deleted,
        "body": body,
    })
}

fn slug(value: &str) -> String {
    value
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}
