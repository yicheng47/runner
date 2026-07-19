// Storage-format serde helpers for the repo layer (impl 0021).
//
// These `#[serde(with)]` modules pin the exact byte formats the legacy
// hand-written mappers and INSERT/UPDATE paths produced, so rows written
// through the repo are indistinguishable from rows written before it:
//
//   - `rfc3339` / `rfc3339_opt`: serialize `Timestamp` via
//     `DateTime::to_rfc3339()` (the `+00:00` offset spelling every current
//     write path uses) and parse back with `str::parse`, which accepts both
//     historical offset spellings (`+00:00` and `Z`).
//   - `json_text` / `json_text_opt`: serialize any `Serialize` value through
//     `serde_json::to_string` into a TEXT column — the same call the legacy
//     write paths made for `args_json` and `env_json`.

pub mod rfc3339 {
    use chrono::{DateTime, Utc};
    use serde::{de, Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &DateTime<Utc>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&v.to_rfc3339())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<DateTime<Utc>, D::Error> {
        let raw = String::deserialize(d)?;
        raw.parse().map_err(de::Error::custom)
    }
}

pub mod rfc3339_opt {
    use chrono::{DateTime, Utc};
    use serde::{de, Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &Option<DateTime<Utc>>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(t) => s.serialize_some(&t.to_rfc3339()),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<DateTime<Utc>>, D::Error> {
        let raw: Option<String> = Option::deserialize(d)?;
        raw.map(|s| s.parse().map_err(de::Error::custom))
            .transpose()
    }
}

// Every JSON TEXT column today is nullable, so production rows go through
// `json_text_opt`; this non-optional twin is spike-proven (repo::spike_tests)
// and waiting for the first NOT NULL JSON column.
#[allow(dead_code)]
pub mod json_text {
    use serde::{de, ser, Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<T: Serialize, S: Serializer>(v: &T, s: S) -> Result<S::Ok, S::Error> {
        let text = serde_json::to_string(v).map_err(ser::Error::custom)?;
        s.serialize_str(&text)
    }

    pub fn deserialize<'de, T: de::DeserializeOwned, D: Deserializer<'de>>(
        d: D,
    ) -> Result<T, D::Error> {
        let text = String::deserialize(d)?;
        serde_json::from_str(&text).map_err(de::Error::custom)
    }
}

pub mod json_text_opt {
    use serde::{de, ser, Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<T: Serialize, S: Serializer>(v: &Option<T>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(inner) => {
                let text = serde_json::to_string(inner).map_err(ser::Error::custom)?;
                s.serialize_some(&text)
            }
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, T: de::DeserializeOwned, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Option<T>, D::Error> {
        let raw: Option<String> = Option::deserialize(d)?;
        raw.map(|text| serde_json::from_str(&text).map_err(de::Error::custom))
            .transpose()
    }
}
