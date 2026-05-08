use std::io::{self, BufRead, IsTerminal, Write};

use anyhow::Result;

use crate::chain::Chain;
use crate::db::{Db, UnknownCounterparty};

pub struct PromptOutcome {
    pub tagged: usize,
    pub skipped: usize,
}

/// Walks unknown counterparties and asks the user to label each one.
/// Returns a count of how many were tagged. If stdin isn't a TTY, returns
/// `None` so the caller can render a non-interactive summary instead.
pub fn prompt_unknowns(
    db: &mut Db,
    items: &[UnknownCounterparty],
) -> Result<Option<PromptOutcome>> {
    if !io::stdin().is_terminal() {
        return Ok(None);
    }

    let stdin = io::stdin();
    let mut lines = stdin.lock();
    let mut stdout = io::stdout();
    let mut tagged = 0usize;
    let mut skipped = 0usize;

    println!(
        "Found {} unknown counterparties. Press Enter to skip a label; type `q` to stop.",
        items.len()
    );
    for (i, item) in items.iter().enumerate() {
        let chain = Chain::from_id(item.chain_id)
            .map(|c| c.name().to_string())
            .unwrap_or_else(|| item.chain_id.to_string());
        println!(
            "\n[{}/{}] {chain}  {addr:#x}  ({n} interaction{plural})",
            i + 1,
            items.len(),
            addr = item.address,
            n = item.interactions,
            plural = if item.interactions == 1 { "" } else { "s" },
        );

        let label = read_line(&mut lines, &mut stdout, "  label> ")?;
        if label.eq_ignore_ascii_case("q") {
            break;
        }
        if label.is_empty() {
            skipped += 1;
            continue;
        }

        let kind = read_line(&mut lines, &mut stdout, "  kind (eoa|contract|protocol)> ")?;
        let kind = if kind.is_empty() {
            "contract"
        } else {
            kind.as_str()
        };

        let chain_for_label = Chain::from_id(item.chain_id).ok_or_else(|| {
            anyhow::anyhow!("unsupported chain id {}; cannot save label", item.chain_id)
        })?;
        db.upsert_label(chain_for_label, item.address, &label, kind)?;
        tagged += 1;
    }

    Ok(Some(PromptOutcome { tagged, skipped }))
}

fn read_line(
    stdin: &mut io::StdinLock<'_>,
    stdout: &mut io::Stdout,
    prompt: &str,
) -> Result<String> {
    stdout.write_all(prompt.as_bytes())?;
    stdout.flush()?;
    let mut buf = String::new();
    stdin.read_line(&mut buf)?;
    Ok(buf.trim().to_string())
}
