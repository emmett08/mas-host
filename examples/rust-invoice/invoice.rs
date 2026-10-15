use mas_host_sdk::{json, Context, Json, Result};

/// Assess integer minor currency units, avoiding rounding during transport.
pub fn assess(_context: &mut Context, input: Json) -> Result<Json> {
    let amount = input["amount_minor"]
        .as_u64()
        .ok_or("amount_minor must be a non-negative 64-bit integer")?;
    let limit = input["limit_minor"]
        .as_u64()
        .ok_or("limit_minor must be a non-negative 64-bit integer")?;
    Ok(json!({"approved": amount <= limit, "amount_minor": amount, "limit_minor": limit}))
}
