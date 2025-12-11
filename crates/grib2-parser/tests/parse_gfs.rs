/// Integration test for parsing real GFS GRIB2 file
mod common;

use bytes::Bytes;
use grib2_parser::Grib2Reader;
use std::fs;

#[test]
fn test_parse_gfs_file() {
    let path = require_test_file!("gfs_sample.grib2");

    let data = fs::read(&path).expect("Failed to read test file");
    let data = Bytes::from(data);

    let mut reader = Grib2Reader::new(data);

    println!("File size: {} bytes", reader.size());

    let mut message_count = 0;
    let mut messages = vec![];
    let mut error_count = 0;

    loop {
        match reader.next_message() {
            Ok(Some(msg)) => {
                message_count += 1;

                if message_count <= 5 {
                    println!(
                        "Message {}: {} at {} (Level: {})",
                        message_count,
                        msg.parameter(),
                        msg.identification.reference_time,
                        msg.level()
                    );
                }

                messages.push(msg);

                if message_count >= 20 {
                    break; // Just test first 20 messages
                }
            }
            Ok(None) => {
                println!("Reached end of file");
                break;
            }
            Err(e) => {
                error_count += 1;
                if error_count <= 3 {
                    println!("Error reading message {}: {}", message_count + 1, e);
                }
                if error_count > 10 {
                    println!("Too many errors, stopping");
                    break;
                }
                break; // Stop at first error for now
            }
        }
    }

    println!("Successfully parsed {} messages", message_count);
    assert!(message_count > 0, "Should have parsed at least one message");

    // Check first message has valid properties
    if let Some(msg) = messages.first() {
        println!("First message details:");
        println!("  Parameter: {}", msg.parameter());
        println!("  Level: {}", msg.level());
        println!("  Valid time: {}", msg.valid_time());
        println!("  Reference time: {}", msg.identification.reference_time);

        assert!(!msg.parameter().is_empty());
        assert!(!msg.level().is_empty());
        // Grid parsing is WIP, so just check that we have basic metadata
        assert_eq!(msg.identification.center, 7); // NCEP
    }
}
