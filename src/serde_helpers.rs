use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub fn serialize_system_time<S>(time: &SystemTime, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let duration = time.duration_since(UNIX_EPOCH).unwrap_or(Duration::from_secs(0));
    duration.as_secs().serialize(serializer)
}

pub fn deserialize_system_time<'de, D>(deserializer: D) -> Result<SystemTime, D::Error>
where
    D: Deserializer<'de>,
{
    let secs = u64::deserialize(deserializer)?;
    Ok(UNIX_EPOCH + Duration::from_secs(secs))
}

pub fn serialize_duration<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    duration.as_millis().serialize(serializer)
}

pub fn deserialize_duration<'de, D>(deserializer: D) -> Result<Duration, D::Error>
where
    D: Deserializer<'de>,
{
    let millis = u128::deserialize(deserializer)?;
    Ok(Duration::from_millis(millis as u64))
}

pub fn serialize_duration_vec<S>(durations: &Vec<Duration>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let millis: Vec<u128> = durations.iter().map(|d| d.as_millis()).collect();
    millis.serialize(serializer)
}

pub fn deserialize_duration_vec<'de, D>(deserializer: D) -> Result<Vec<Duration>, D::Error>
where
    D: Deserializer<'de>,
{
    let millis: Vec<u128> = Vec::deserialize(deserializer)?;
    Ok(millis.into_iter().map(|m| Duration::from_millis(m as u64)).collect())
}

pub mod systemtime {
    use super::*;
    
    pub fn serialize<S>(time: &SystemTime, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serialize_system_time(time, serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<SystemTime, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_system_time(deserializer)
    }
}