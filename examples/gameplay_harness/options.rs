//! Command-line surface: option parsing, seed handling, and usage text.

use std::path::PathBuf;

use crate::*;

pub fn split_flag_value(arg: &str) -> Option<(&str, &str)> {
    let (flag, value) = arg.split_once('=')?;
    if flag.starts_with("--") {
        Some((flag, value))
    } else {
        None
    }
}

pub fn parse_seed_value(raw: &str) -> Result<u64, HarnessCliError> {
    let value = raw.to_owned();
    let invalid = || HarnessCliError::InvalidValue {
        flag: "--seed",
        value: value.clone(),
    };
    // Accepted forms are exactly `0x[hex]` or all-decimal digits; anything else is a typo
    // and must be rejected rather than silently reinterpreted in another radix.
    if let Some(hex) = raw.strip_prefix("0x").or_else(|| raw.strip_prefix("0X")) {
        u64::from_str_radix(hex, 16).map_err(|_| invalid())
    } else if raw.chars().all(|c| c.is_ascii_digit()) && !raw.is_empty() {
        raw.parse::<u64>().map_err(|_| invalid())
    } else {
        Err(invalid())
    }
}

pub fn parse_options(
    arguments: impl IntoIterator<Item = String>,
) -> Result<Option<HarnessOptions>, HarnessCliError> {
    let mut arguments = arguments.into_iter();
    let mut mode = HarnessMode::Smoke;
    let mut samples = DEFAULT_BATCH_SAMPLES;
    let mut seed = DEFAULT_SEED;
    let mut strategy = None;
    let mut strategy_was_passed = false;
    let mut samples_were_explicit = false;
    let mut artifact_dir: Option<PathBuf> = None;
    while let Some(argument) = arguments.next() {
        // Support both --flag value and --flag=value forms.
        let (flag, inline_value) = if let Some((f, v)) = split_flag_value(&argument) {
            (f.to_owned(), Some(v.to_owned()))
        } else {
            (argument.clone(), None)
        };
        match flag.as_str() {
            "--help" | "-h" => {
                print_usage();
                return Ok(None);
            }
            "--mode" => {
                let value = inline_value.unwrap_or_else(|| arguments.next().unwrap_or_default());
                if value.is_empty() {
                    return Err(HarnessCliError::MissingValue { flag: "--mode" });
                }
                mode = HarnessMode::parse(&value)?;
            }
            "--samples" => {
                samples_were_explicit = true;
                let value = inline_value.unwrap_or_else(|| arguments.next().unwrap_or_default());
                if value.is_empty() {
                    return Err(HarnessCliError::MissingValue { flag: "--samples" });
                }
                samples = value
                    .parse::<u64>()
                    .map_err(|_| HarnessCliError::InvalidValue {
                        flag: "--samples",
                        value: value.clone(),
                    })?;
                if !(1..=MAX_BATCH_SAMPLES).contains(&samples) {
                    return Err(HarnessCliError::SampleCountOutOfRange { value: samples });
                }
            }
            "--seed" => {
                let value = inline_value.unwrap_or_else(|| arguments.next().unwrap_or_default());
                if value.is_empty() {
                    return Err(HarnessCliError::MissingValue { flag: "--seed" });
                }
                seed = parse_seed_value(&value)?;
            }
            "--strategy" => {
                let value = inline_value.unwrap_or_else(|| arguments.next().unwrap_or_default());
                if value.is_empty() {
                    return Err(HarnessCliError::MissingValue { flag: "--strategy" });
                }
                strategy = Strategy::parse(&value)?;
                strategy_was_passed = true;
            }
            "--artifact-dir" => {
                let value = inline_value.unwrap_or_else(|| arguments.next().unwrap_or_default());
                if value.is_empty() {
                    return Err(HarnessCliError::MissingValue {
                        flag: "--artifact-dir",
                    });
                }
                if value.is_empty() {
                    return Err(HarnessCliError::InvalidValue {
                        flag: "--artifact-dir",
                        value: value.clone(),
                    });
                }
                artifact_dir = Some(PathBuf::from(value));
            }
            _ => {
                return Err(HarnessCliError::UnsupportedArgument { argument });
            }
        }
    }
    if mode == HarnessMode::Smoke {
        if samples_were_explicit && samples != 1 {
            return Err(HarnessCliError::SmokeSampleCount { value: samples });
        }
        samples = 1;
    } else if strategy_was_passed {
        return Err(HarnessCliError::StrategyOnlyInSmoke);
    }
    Ok(Some(HarnessOptions {
        mode,
        samples,
        seed,
        strategy,
        artifact_dir,
    }))
}

pub fn print_usage() {
    println!(
        "Usage: cargo run --example gameplay_harness -- [--mode smoke|full] [--strategy all|rush|press|recon] [--samples 1..={MAX_BATCH_SAMPLES}] [--seed HEX|DEC] [--artifact-dir DIR]"
    );
    println!("  smoke  Fast canonical-path check for the local gate and iteration (default).");
    println!("         --strategy rush|press|recon focuses one branch; default is all.");
    println!("         --seed accepts 0xHEX or decimal; --flag=value form also supported.");
    println!("  full   Narrative session, legal check, matched batch, and sensitivity report.");
    println!("         --artifact-dir writes per-run JSON artifacts (default: target/harness/).");
}
