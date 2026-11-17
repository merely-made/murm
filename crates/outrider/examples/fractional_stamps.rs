//! Line-oriented stock comparison for quantized costs, thresholds and delivery announces.
use outrider::{DeliveryAnnounce, NodePolicy, PropagationAnnounce, StampCost};
use retinue::stamp::value_of_digest;
use std::io::{self, BufRead};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    for line in io::stdin().lock().lines() {
        let line = line?;
        let parts: Vec<_> = line.split_whitespace().collect();
        match parts.as_slice() {
            ["COST", value] => match StampCost::from_f64(value.parse()?) {
                Some(cost) => println!("COST {}", cost.to_fp()),
                None => println!("INVALID"),
            },
            ["DIGEST", fp, digest] => {
                let digest: [u8; 32] = hex::decode(digest)?
                    .try_into()
                    .map_err(|_| "digest length")?;
                let cost = StampCost::from_fp(fp.parse()?);
                println!(
                    "VALID {} VALUE {}",
                    cost.accepts_digest(&digest),
                    value_of_digest(&digest)
                );
            }
            ["ANNOUNCE", bytes] => match DeliveryAnnounce::decode(&hex::decode(bytes)?)?.stamp_cost
            {
                Some(cost) => println!("COST {}", cost.to_fp()),
                None => println!("NONE"),
            },
            ["PROPAGATION", bytes] => match PropagationAnnounce::decode(&hex::decode(bytes)?) {
                Ok(announce) => println!(
                    "COSTS {} {} {} FLOOR {}",
                    announce.costs.propagation.quantized().to_fp(),
                    announce.costs.flexibility.quantized().to_fp(),
                    announce.costs.peering.quantized().to_fp(),
                    NodePolicy::from_announce(&announce).stamp_floor().to_fp()
                ),
                Err(_) => println!("INVALID"),
            },
            _ => return Err("expected COST, DIGEST or ANNOUNCE".into()),
        }
    }
    Ok(())
}
