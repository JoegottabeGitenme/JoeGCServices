use grib2_parser::Grib2Reader;
use std::fs;
use bytes::Bytes;
use std::collections::HashMap;

#[test]
fn test_mrms_data_distribution() {
    let path = "/tmp/mrms_refl_full.grib2";
    let data = fs::read(path).expect("Failed to read test file");
    let mut reader = Grib2Reader::new(Bytes::from(data));

    match reader.next_message() {
        Ok(Some(msg)) => {
            let (nj, ni) = msg.grid_dims();
            let values = msg.unpack_data().expect("Failed to unpack");
            
            println!("Grid: {}x{} = {} points", ni, nj, values.len());
            
            // Count value categories
            let mut missing_99 = 0;
            let mut missing_999 = 0;
            let mut negative = 0;
            let mut zero_to_10 = 0;
            let mut ten_to_30 = 0;
            let mut above_30 = 0;
            
            for &v in &values {
                if v <= -99.0 {
                    if v <= -900.0 {
                        missing_999 += 1;
                    } else {
                        missing_99 += 1;
                    }
                } else if v < 0.0 {
                    negative += 1;
                } else if v < 10.0 {
                    zero_to_10 += 1;
                } else if v < 30.0 {
                    ten_to_30 += 1;
                } else {
                    above_30 += 1;
                }
            }
            
            println!("\nValue distribution:");
            println!("  Missing (-999): {}", missing_999);
            println!("  Missing (-99): {}", missing_99);
            println!("  Negative (< 0 dBZ): {}", negative);
            println!("  0-10 dBZ: {}", zero_to_10);
            println!("  10-30 dBZ: {}", ten_to_30);
            println!("  30+ dBZ: {}", above_30);
            
            // Check a specific row for data pattern
            let mid_row = (nj / 2) as usize;
            let row_start = mid_row * ni as usize;
            let row_end = row_start + ni as usize;
            let row_data = &values[row_start..row_end];
            
            // Find runs of non-missing data
            let mut in_data = false;
            let mut run_start = 0;
            let mut runs = Vec::new();
            
            for (i, &v) in row_data.iter().enumerate() {
                let is_missing = v <= -90.0;
                if !is_missing && !in_data {
                    in_data = true;
                    run_start = i;
                } else if is_missing && in_data {
                    runs.push((run_start, i - 1));
                    in_data = false;
                }
            }
            if in_data {
                runs.push((run_start, row_data.len() - 1));
            }
            
            println!("\nMiddle row ({}) data runs (non-missing regions):", mid_row);
            for (start, end) in &runs {
                let span = end - start + 1;
                // Convert to lon
                let lon_start = -129.995 + (*start as f64) * 0.01;
                let lon_end = -129.995 + (*end as f64) * 0.01;
                println!("  cols {}-{} ({} pts) = lon {:.2}° to {:.2}°", start, end, span, lon_start, lon_end);
            }
        }
        Ok(None) => println!("No messages"),
        Err(e) => println!("Error: {}", e),
    }
}
