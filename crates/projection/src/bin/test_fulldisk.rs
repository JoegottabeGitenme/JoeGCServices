use projection::Geostationary;

fn main() {
    // Test GOES-19 (East)
    let proj19 = Geostationary::goes19_fulldisk();
    let (min_lon, min_lat, max_lon, max_lat) = proj19.geographic_bounds();
    println!(
        "GOES-19 Full Disk bounds: lon {:.2} to {:.2}, lat {:.2} to {:.2}",
        min_lon, max_lon, min_lat, max_lat
    );

    // Test GOES-18 (West)
    let proj18 = Geostationary::goes18_fulldisk();
    let (min_lon18, min_lat18, max_lon18, max_lat18) = proj18.geographic_bounds();
    println!(
        "GOES-18 Full Disk bounds: lon {:.2} to {:.2}, lat {:.2} to {:.2}",
        min_lon18, max_lon18, min_lat18, max_lat18
    );

    // Check a few points for GOES-18
    println!("\nGOES-18 sample points:");
    let center = proj18.grid_to_geo(2712.0, 2712.0);
    println!("  Center (2712, 2712): {:?}", center);

    // Check extreme west (should be near date line)
    for i in [0, 100, 500, 1000, 2000, 2712, 3500, 4500, 5000, 5400] {
        if let Some((lat, lon)) = proj18.grid_to_geo(i as f64, 2712.0) {
            println!("  Horizontal i={}: lon={:.2}, lat={:.2}", i, lon, lat);
        }
    }
}
