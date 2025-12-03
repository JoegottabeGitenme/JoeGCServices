# wgrib2 Quick Reference Guide

## Purpose

`wgrib2` is the authoritative tool for working with GRIB2 files. We use it as **ground truth** to verify our parser's correctness.

## Installation

```bash
# Arch Linux
sudo pacman -S wgrib2

# Ubuntu/Debian
sudo apt install wgrib2

# From source
wget https://www.ftp.cpc.ncep.noaa.gov/wd51we/wgrib2/wgrib2.tgz
tar -xzf wgrib2.tgz
cd grib2
make
```

## Basic Commands

### List all messages in a GRIB2 file
```bash
wgrib2 testdata/gfs_sample.grib2
```

Output format: `message:offset:date:variable:level:forecast`

### Get detailed inventory
```bash
wgrib2 -s testdata/gfs_sample.grib2
```

Shows: parameter name, level, forecast time, grid info

### Extract specific parameters
```bash
# Show only temperature messages
wgrib2 testdata/gfs_sample.grib2 | grep ":TMP:"

# Show only pressure messages  
wgrib2 testdata/gfs_sample.grib2 | grep ":PRMSL:"
```

### Get grid information
```bash
wgrib2 -grid testdata/gfs_sample.grib2
```

Shows: grid type, dimensions, spacing, projection

### Dump metadata
```bash
wgrib2 -V testdata/gfs_sample.grib2
```

Verbose output with all section details

### Extract values
```bash
# Dump all grid point values
wgrib2 -text output.txt testdata/gfs_sample.grib2

# Dump in CSV format
wgrib2 -csv output.csv testdata/gfs_sample.grib2

# Dump specific message (message 1)
wgrib2 -d 1 -text values.txt testdata/gfs_sample.grib2
```

## Verification Workflow

### 1. Check Parameter Identification
```bash
wgrib2 -s testdata/gfs_sample.grib2 | head -5
```

Compare with our parser:
```bash
cargo run --example list_messages testdata/gfs_sample.grib2
```

**They should match!**

### 2. Check Grid Dimensions
```bash
wgrib2 -grid testdata/gfs_sample.grib2 | head -10
```

Compare with our parser output - dimensions should be identical.

### 3. Check Data Values
```bash
# Extract first 100 values from message 1
wgrib2 -d 1 -text /tmp/wgrib_values.txt testdata/gfs_sample.grib2
head -100 /tmp/wgrib_values.txt
```

Compare with:
```bash
cargo run --example test_rendering testdata/gfs_sample.grib2
```

Values should be within floating-point precision.

### 4. Check Compression Type
```bash
wgrib2 -V testdata/gfs_sample.grib2 | grep -i "packing\|template"
```

Look for:
- `Data Template 5.0` = Simple packing
- `Data Template 5.15` = PNG compression  
- `Data Template 5.40` = JPEG 2000

## Common Use Cases

### Split multi-message file
```bash
# Extract each message to separate file
wgrib2 testdata/gfs_sample.grib2 -split grib_msg_
```

Creates: `grib_msg_001.grb`, `grib_msg_002.grb`, etc.

### Convert to NetCDF
```bash
wgrib2 testdata/gfs_sample.grib2 -netcdf output.nc
```

### Filter by parameter
```bash
# Extract only temperature to new file
wgrib2 testdata/gfs_sample.grib2 -match ":TMP:" -grib temp_only.grb2
```

### Get statistics
```bash
# Min, max, mean for each message
wgrib2 -stats testdata/gfs_sample.grib2
```

## Debugging Our Parser

### When values don't match:

1. **Check parameter code**:
   ```bash
   wgrib2 -V file.grib2 | grep -A 5 "Product Definition Section"
   ```
   
   Look for:
   - Parameter category
   - Parameter number
   - Level type

2. **Check grid definition**:
   ```bash
   wgrib2 -V file.grib2 | grep -A 10 "Grid Definition Section"
   ```

3. **Check data representation**:
   ```bash
   wgrib2 -V file.grib2 | grep -A 10 "Data Representation Section"
   ```
   
   Look for:
   - Packing method
   - Reference value
   - Binary/decimal scale factors

### Compare byte-by-byte:

```bash
# Our parser
cargo run --example inspect_grib2 file.grib2 > our_output.txt

# wgrib2
wgrib2 -V file.grib2 > wgrib_output.txt

# Compare
diff our_output.txt wgrib_output.txt
```

## Integration with Tests

### Add to CI/CD
```bash
#!/bin/bash
# tests/verify_parsing.sh

GRIB_FILE="testdata/gfs_sample.grib2"

# Get wgrib2 output
wgrib2 -s $GRIB_FILE > /tmp/wgrib2_output.txt

# Get our parser output
cargo run --example list_messages $GRIB_FILE > /tmp/our_output.txt

# Compare key fields (parameter, level, grid size)
# Exit with error if mismatch
```

### Automated Testing
```rust
#[test]
fn test_against_wgrib2() {
    let wgrib2_output = std::process::Command::new("wgrib2")
        .arg("-s")
        .arg("testdata/gfs_sample.grib2")
        .output()
        .expect("wgrib2 should be installed");
    
    let our_output = list_messages("testdata/gfs_sample.grib2");
    
    // Parse and compare...
}
```

## Quick Sanity Checks

Before committing GRIB parsing changes:

```bash
# 1. Parameters match
wgrib2 -s testdata/*.grib2 | awk -F: '{print $4}' | sort -u
cargo run --example list_messages testdata/*.grib2 | grep "Parameter:" | sort -u

# 2. Grid sizes match  
wgrib2 -grid testdata/*.grib2 | grep "grid template"
cargo run --example list_messages testdata/*.grib2 | grep "grid:"

# 3. Values are reasonable
wgrib2 -stats testdata/gfs_sample.grib2
cargo run --example test_rendering testdata/gfs_sample.grib2 | grep "Min:\|Max:"
```

## Resources

- [wgrib2 Documentation](https://www.cpc.ncep.noaa.gov/products/wesley/wgrib2/)
- [GRIB2 Tables](https://www.nco.ncep.noaa.gov/pmb/docs/grib2/grib2_doc/)
- [NOAA GRIB2 Guide](https://www.nco.ncep.noaa.gov/pmb/docs/grib2/)

---

**Golden Rule**: If wgrib2 and our parser disagree, wgrib2 is right. Fix our parser.
