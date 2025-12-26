window.BENCHMARK_DATA = {
  "lastUpdate": 1766762054226,
  "repoUrl": "https://github.com/JoegottabeGitenme/JoeGCServices",
  "entries": {
    "Rust Benchmarks": [
      {
        "commit": {
          "author": {
            "email": "JoegottabeGitenme@users.noreply.github.com",
            "name": "JoegottabeGitenme",
            "username": "JoegottabeGitenme"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "935d4e13db7503c13c476b3cd1030cda33960b77",
          "message": "Merge pull request #5 from JoegottabeGitenme/feature/wms-improvements\n\nFeature/wms improvements",
          "timestamp": "2025-12-23T12:44:34-07:00",
          "tree_id": "5ea73d2e6969bd3ac68e220c95012d38a5a5da30",
          "url": "https://github.com/JoegottabeGitenme/JoeGCServices/commit/935d4e13db7503c13c476b3cd1030cda33960b77"
        },
        "date": 1766519125589,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "barb_full_pipeline/render_and_png",
            "value": 4639217.520479216,
            "range": "± 5952.68",
            "unit": "ns"
          },
          {
            "name": "barb_positions/pixel_grid/tile_1024",
            "value": 1662.6741885617184,
            "range": "± 2.88",
            "unit": "ns"
          },
          {
            "name": "barb_positions/pixel_grid/tile_256",
            "value": 399.0142396403697,
            "range": "± 0.61",
            "unit": "ns"
          },
          {
            "name": "barb_positions/pixel_grid/tile_256_dense",
            "value": 549.7046968716508,
            "range": "± 2.26",
            "unit": "ns"
          },
          {
            "name": "barb_positions/pixel_grid/tile_256_sparse",
            "value": 187.1183752273211,
            "range": "± 0.12",
            "unit": "ns"
          },
          {
            "name": "barb_positions/pixel_grid/tile_512",
            "value": 600.3919216112683,
            "range": "± 1.07",
            "unit": "ns"
          },
          {
            "name": "barb_positions_geographic/geographic/conus_large",
            "value": 2766.4033107902355,
            "range": "± 1.12",
            "unit": "ns"
          },
          {
            "name": "barb_positions_geographic/geographic/conus_z5",
            "value": 2856.456552474101,
            "range": "± 1.87",
            "unit": "ns"
          },
          {
            "name": "barb_positions_geographic/geographic/local_z11",
            "value": 11798.295431455395,
            "range": "± 10.81",
            "unit": "ns"
          },
          {
            "name": "barb_positions_geographic/geographic/region_z8",
            "value": 1920.1330082022894,
            "range": "± 3.17",
            "unit": "ns"
          },
          {
            "name": "barb_size_impact/size/108",
            "value": 1094926.9515797466,
            "range": "± 607.40",
            "unit": "ns"
          },
          {
            "name": "barb_size_impact/size/24",
            "value": 475087.7433587474,
            "range": "± 368.36",
            "unit": "ns"
          },
          {
            "name": "barb_size_impact/size/40",
            "value": 510782.7760195229,
            "range": "± 586.46",
            "unit": "ns"
          },
          {
            "name": "barb_size_impact/size/64",
            "value": 601051.4856122704,
            "range": "± 256.64",
            "unit": "ns"
          },
          {
            "name": "color_functions/pressure_color_100",
            "value": 539.0752762844107,
            "range": "± 0.43",
            "unit": "ns"
          },
          {
            "name": "color_functions/temperature_color_100",
            "value": 724.4830005321907,
            "range": "± 0.38",
            "unit": "ns"
          },
          {
            "name": "color_functions/wind_speed_color_40",
            "value": 199.15253860100023,
            "range": "± 0.12",
            "unit": "ns"
          },
          {
            "name": "connect_segments/smooth/128x128_993seg",
            "value": 545472.9379362835,
            "range": "± 189.10",
            "unit": "ns"
          },
          {
            "name": "connect_segments/smooth/256x256_2017seg",
            "value": 2169745.2679166673,
            "range": "± 606.15",
            "unit": "ns"
          },
          {
            "name": "full_contour_pipeline/contour_256x256_7levels",
            "value": 8765924.519022124,
            "range": "± 5152.19",
            "unit": "ns"
          },
          {
            "name": "full_contour_pipeline/contour_256x256_dense",
            "value": 68974228.85,
            "range": "± 20034.98",
            "unit": "ns"
          },
          {
            "name": "full_pipeline/temperature_tile_256x256",
            "value": 2961243.8364705876,
            "range": "± 935.15",
            "unit": "ns"
          },
          {
            "name": "full_pipeline/temperature_tile_512x512",
            "value": 11777707.933999998,
            "range": "± 3357.97",
            "unit": "ns"
          },
          {
            "name": "generate_all_contours/20_levels/128x128",
            "value": 5696038.0468075,
            "range": "± 3171.12",
            "unit": "ns"
          },
          {
            "name": "generate_all_contours/20_levels/256x256",
            "value": 21607559.295710474,
            "range": "± 8305.14",
            "unit": "ns"
          },
          {
            "name": "generate_all_contours/4_levels/128x128",
            "value": 1169360.4556605457,
            "range": "± 850.60",
            "unit": "ns"
          },
          {
            "name": "generate_all_contours/4_levels/256x256",
            "value": 4432397.226970718,
            "range": "± 1514.78",
            "unit": "ns"
          },
          {
            "name": "generate_contour_levels/levels/0-100_by_10",
            "value": 87.11634818538484,
            "range": "± 0.15",
            "unit": "ns"
          },
          {
            "name": "generate_contour_levels/levels/0-100_by_2",
            "value": 316.13949558755024,
            "range": "± 0.52",
            "unit": "ns"
          },
          {
            "name": "generate_contour_levels/levels/0-100_by_5",
            "value": 171.49758940750283,
            "range": "± 0.28",
            "unit": "ns"
          },
          {
            "name": "generate_contour_levels/levels/neg50-50_by_5",
            "value": 170.04518237985155,
            "range": "± 0.25",
            "unit": "ns"
          },
          {
            "name": "generate_contour_levels/levels/pressure_4hPa",
            "value": 306.73341543213724,
            "range": "± 0.36",
            "unit": "ns"
          },
          {
            "name": "goes_color/ir_enhanced/1024x1024",
            "value": 5698197.87,
            "range": "± 1805.94",
            "unit": "ns"
          },
          {
            "name": "goes_color/ir_enhanced/256x256",
            "value": 330345.2026035377,
            "range": "± 142.52",
            "unit": "ns"
          },
          {
            "name": "goes_color/ir_enhanced/512x512",
            "value": 1428777.543743859,
            "range": "± 981.02",
            "unit": "ns"
          },
          {
            "name": "goes_color/visible_grayscale/1024x1024",
            "value": 9852003.523333333,
            "range": "± 2419.82",
            "unit": "ns"
          },
          {
            "name": "goes_color/visible_grayscale/256x256",
            "value": 613773.2887138363,
            "range": "± 228.73",
            "unit": "ns"
          },
          {
            "name": "goes_color/visible_grayscale/512x512",
            "value": 2466799.723809524,
            "range": "± 586.35",
            "unit": "ns"
          },
          {
            "name": "goes_pipeline/color_and_png_only_256x256",
            "value": 1956481.7516754603,
            "range": "± 1612.14",
            "unit": "ns"
          },
          {
            "name": "goes_pipeline/ir_tile_256x256",
            "value": 15552839.2675,
            "range": "± 6157.56",
            "unit": "ns"
          },
          {
            "name": "goes_pipeline/resample_only_256x256",
            "value": 13568384.2425,
            "range": "± 3147.37",
            "unit": "ns"
          },
          {
            "name": "goes_pipeline/visible_tile_256x256",
            "value": 15156952.015,
            "range": "± 7406.81",
            "unit": "ns"
          },
          {
            "name": "goes_png/encode/256x256",
            "value": 1580693.467893488,
            "range": "± 1145.50",
            "unit": "ns"
          },
          {
            "name": "goes_png/encode/512x512",
            "value": 6463221.2975,
            "range": "± 1817.24",
            "unit": "ns"
          },
          {
            "name": "goes_projection/geo_to_grid/1048576",
            "value": 172213294.39,
            "range": "± 37713.52",
            "unit": "ns"
          },
          {
            "name": "goes_projection/geo_to_grid/262144",
            "value": 43053917.67,
            "range": "± 8540.34",
            "unit": "ns"
          },
          {
            "name": "goes_projection/geo_to_grid/65536",
            "value": 10762607.943999995,
            "range": "± 1624.30",
            "unit": "ns"
          },
          {
            "name": "goes_projection/geo_to_scan/65536",
            "value": 10730754.015999995,
            "range": "± 3374.64",
            "unit": "ns"
          },
          {
            "name": "goes_resample/bilinear_only/central_us_z7",
            "value": 678947.4789308778,
            "range": "± 496.34",
            "unit": "ns"
          },
          {
            "name": "goes_resample/bilinear_only/full_conus_z4",
            "value": 678697.8087989176,
            "range": "± 308.17",
            "unit": "ns"
          },
          {
            "name": "goes_resample/bilinear_only/full_conus_z4_512",
            "value": 2698094.336842105,
            "range": "± 600.50",
            "unit": "ns"
          },
          {
            "name": "goes_resample/bilinear_only/kansas_city_z10",
            "value": 678546.135277414,
            "range": "± 652.04",
            "unit": "ns"
          },
          {
            "name": "goes_resample/with_projection/central_us_z7",
            "value": 13776319.925,
            "range": "± 3205.12",
            "unit": "ns"
          },
          {
            "name": "goes_resample/with_projection/full_conus_z4",
            "value": 13544278.41,
            "range": "± 7367.10",
            "unit": "ns"
          },
          {
            "name": "goes_resample/with_projection/full_conus_z4_512",
            "value": 54471923.79,
            "range": "± 63040.37",
            "unit": "ns"
          },
          {
            "name": "goes_resample/with_projection/kansas_city_z10",
            "value": 13732667.68,
            "range": "± 1596.27",
            "unit": "ns"
          },
          {
            "name": "line_width_impact/width/1",
            "value": 6414883.063922903,
            "range": "± 2409.17",
            "unit": "ns"
          },
          {
            "name": "line_width_impact/width/2",
            "value": 9090137.251469847,
            "range": "± 3609.23",
            "unit": "ns"
          },
          {
            "name": "line_width_impact/width/4",
            "value": 9424378.494647447,
            "range": "± 2351.86",
            "unit": "ns"
          },
          {
            "name": "line_width_impact/width/8",
            "value": 9960270.297749942,
            "range": "± 3361.70",
            "unit": "ns"
          },
          {
            "name": "march_squares/noisy_single_level/128x128",
            "value": 199628.11517290713,
            "range": "± 153.47",
            "unit": "ns"
          },
          {
            "name": "march_squares/noisy_single_level/256x256",
            "value": 785508.1570511982,
            "range": "± 425.13",
            "unit": "ns"
          },
          {
            "name": "march_squares/noisy_single_level/512x512",
            "value": 3170119.94375,
            "range": "± 3885.50",
            "unit": "ns"
          },
          {
            "name": "march_squares/noisy_single_level/64x64",
            "value": 54209.34989645428,
            "range": "± 40.05",
            "unit": "ns"
          },
          {
            "name": "march_squares/smooth_single_level/128x128",
            "value": 157368.56548937646,
            "range": "± 98.92",
            "unit": "ns"
          },
          {
            "name": "march_squares/smooth_single_level/256x256",
            "value": 573599.5881877627,
            "range": "± 300.43",
            "unit": "ns"
          },
          {
            "name": "march_squares/smooth_single_level/512x512",
            "value": 2175903.2134782607,
            "range": "± 407.48",
            "unit": "ns"
          },
          {
            "name": "march_squares/smooth_single_level/64x64",
            "value": 46197.13335708418,
            "range": "± 39.49",
            "unit": "ns"
          },
          {
            "name": "netcdf_io_pattern/current_pattern_with_sync",
            "value": 6878696.56875,
            "range": "± 111252.58",
            "unit": "ns"
          },
          {
            "name": "netcdf_io_pattern/no_sync_pattern",
            "value": 2235353.9065217385,
            "range": "± 1585.98",
            "unit": "ns"
          },
          {
            "name": "netcdf_io_pattern/sequential_3x_operations",
            "value": 6693362.4775,
            "range": "± 3502.72",
            "unit": "ns"
          },
          {
            "name": "png_encoding/create_png/1024x1024",
            "value": 27535957.015,
            "range": "± 22715.07",
            "unit": "ns"
          },
          {
            "name": "png_encoding/create_png/256x256",
            "value": 1667529.9635970218,
            "range": "± 2222.83",
            "unit": "ns"
          },
          {
            "name": "png_encoding/create_png/512x512",
            "value": 6896697.6525,
            "range": "± 2353.68",
            "unit": "ns"
          },
          {
            "name": "projection_lut/compute_lut_z5",
            "value": 13322471.8475,
            "range": "± 1355.62",
            "unit": "ns"
          },
          {
            "name": "projection_lut/compute_lut_z7",
            "value": 14030218.6325,
            "range": "± 1525.53",
            "unit": "ns"
          },
          {
            "name": "projection_lut/on_the_fly/z5_central_conus",
            "value": 14426943.155,
            "range": "± 8910.35",
            "unit": "ns"
          },
          {
            "name": "projection_lut/on_the_fly/z6_midwest",
            "value": 15036832.09,
            "range": "± 1821.40",
            "unit": "ns"
          },
          {
            "name": "projection_lut/on_the_fly/z7_detailed",
            "value": 15046464.255,
            "range": "± 2063.15",
            "unit": "ns"
          },
          {
            "name": "projection_lut/with_lut/z5_central_conus",
            "value": 779463.5035092621,
            "range": "± 676.16",
            "unit": "ns"
          },
          {
            "name": "projection_lut/with_lut/z6_midwest",
            "value": 785889.6076527601,
            "range": "± 355.64",
            "unit": "ns"
          },
          {
            "name": "projection_lut/with_lut/z7_detailed",
            "value": 777055.3484218541,
            "range": "± 560.50",
            "unit": "ns"
          },
          {
            "name": "render_contours_to_canvas/4_levels/256x256",
            "value": 3480188.1736801513,
            "range": "± 1688.97",
            "unit": "ns"
          },
          {
            "name": "render_contours_to_canvas/4_levels/512x512",
            "value": 4528081.4194518,
            "range": "± 6974.33",
            "unit": "ns"
          },
          {
            "name": "render_grid/generic/1024x1024",
            "value": 4533680.235833333,
            "range": "± 799.76",
            "unit": "ns"
          },
          {
            "name": "render_grid/generic/256x256",
            "value": 228001.31405319768,
            "range": "± 173.90",
            "unit": "ns"
          },
          {
            "name": "render_grid/generic/512x512",
            "value": 1077268.935708867,
            "range": "± 1626.22",
            "unit": "ns"
          },
          {
            "name": "render_other/humidity",
            "value": 736190.6506169996,
            "range": "± 748.73",
            "unit": "ns"
          },
          {
            "name": "render_other/pressure",
            "value": 691397.5859716365,
            "range": "± 518.00",
            "unit": "ns"
          },
          {
            "name": "render_other/wind_speed",
            "value": 556355.1845173545,
            "range": "± 212.88",
            "unit": "ns"
          },
          {
            "name": "render_temperature/celsius/1024x1024",
            "value": 12486728.588000007,
            "range": "± 1730.75",
            "unit": "ns"
          },
          {
            "name": "render_temperature/celsius/256x256",
            "value": 789884.35951045,
            "range": "± 291.77",
            "unit": "ns"
          },
          {
            "name": "render_temperature/celsius/512x512",
            "value": 3141174.363125,
            "range": "± 1496.37",
            "unit": "ns"
          },
          {
            "name": "render_wind_barbs/uniform_wind/tile_256_default",
            "value": 473172.01640795905,
            "range": "± 279.11",
            "unit": "ns"
          },
          {
            "name": "render_wind_barbs/uniform_wind/tile_256_dense",
            "value": 1516495.7490822368,
            "range": "± 6711.44",
            "unit": "ns"
          },
          {
            "name": "render_wind_barbs/uniform_wind/tile_512",
            "value": 1897949.135934192,
            "range": "± 1380.96",
            "unit": "ns"
          },
          {
            "name": "render_wind_barbs/varied_wind_256",
            "value": 4114191.8317579753,
            "range": "± 3431.54",
            "unit": "ns"
          },
          {
            "name": "render_wind_barbs_aligned/aligned/bbox_10deg",
            "value": 3498192.7250817665,
            "range": "± 11260.53",
            "unit": "ns"
          },
          {
            "name": "render_wind_barbs_aligned/aligned/bbox_3deg",
            "value": 2779166.5201905826,
            "range": "± 2235.09",
            "unit": "ns"
          },
          {
            "name": "render_wind_barbs_aligned/aligned/bbox_conus",
            "value": 1396122.9589164557,
            "range": "± 1557.27",
            "unit": "ns"
          },
          {
            "name": "resample_grid/GFS_to_large/bilinear",
            "value": 2144358.91375,
            "range": "± 808.11",
            "unit": "ns"
          },
          {
            "name": "resample_grid/GFS_to_tile/bilinear",
            "value": 544148.9072326731,
            "range": "± 286.63",
            "unit": "ns"
          },
          {
            "name": "resample_grid/GOES_to_tile/bilinear",
            "value": 531427.1006905976,
            "range": "± 308.43",
            "unit": "ns"
          },
          {
            "name": "resample_grid/MRMS_to_tile/bilinear",
            "value": 566556.3329640214,
            "range": "± 295.78",
            "unit": "ns"
          },
          {
            "name": "resample_grid/downscale_2x/bilinear",
            "value": 539271.8463956423,
            "range": "± 603.62",
            "unit": "ns"
          },
          {
            "name": "resample_grid/upscale_2x/bilinear",
            "value": 2135894.2908333335,
            "range": "± 331.23",
            "unit": "ns"
          },
          {
            "name": "smooth_contour/1_passes/100_points",
            "value": 444.1947687284956,
            "range": "± 0.82",
            "unit": "ns"
          },
          {
            "name": "smooth_contour/1_passes/10_points",
            "value": 67.62366149336798,
            "range": "± 0.09",
            "unit": "ns"
          },
          {
            "name": "smooth_contour/1_passes/500_points",
            "value": 2048.633061247985,
            "range": "± 3.63",
            "unit": "ns"
          },
          {
            "name": "smooth_contour/1_passes/50_points",
            "value": 226.1455916913905,
            "range": "± 0.17",
            "unit": "ns"
          },
          {
            "name": "smooth_contour/2_passes/100_points",
            "value": 1256.4491213830029,
            "range": "± 1.08",
            "unit": "ns"
          },
          {
            "name": "smooth_contour/2_passes/10_points",
            "value": 155.69873311410944,
            "range": "± 0.12",
            "unit": "ns"
          },
          {
            "name": "smooth_contour/2_passes/500_points",
            "value": 5875.42809889044,
            "range": "± 9.46",
            "unit": "ns"
          },
          {
            "name": "smooth_contour/2_passes/50_points",
            "value": 654.701865547575,
            "range": "± 0.45",
            "unit": "ns"
          },
          {
            "name": "smooth_contour/3_passes/100_points",
            "value": 2816.504375797505,
            "range": "± 2.30",
            "unit": "ns"
          },
          {
            "name": "smooth_contour/3_passes/10_points",
            "value": 317.95634648434674,
            "range": "± 0.42",
            "unit": "ns"
          },
          {
            "name": "smooth_contour/3_passes/500_points",
            "value": 13580.669253202203,
            "range": "± 17.44",
            "unit": "ns"
          },
          {
            "name": "smooth_contour/3_passes/50_points",
            "value": 1469.1703597980074,
            "range": "± 4.57",
            "unit": "ns"
          },
          {
            "name": "subset_grid/americas/GFS",
            "value": 317545.2412487296,
            "range": "± 222.61",
            "unit": "ns"
          },
          {
            "name": "subset_grid/conus/GFS",
            "value": 53975.467333825545,
            "range": "± 47.75",
            "unit": "ns"
          },
          {
            "name": "subset_grid/europe/GFS",
            "value": 44002.000574326696,
            "range": "± 48.84",
            "unit": "ns"
          },
          {
            "name": "subset_grid/small_region/GFS",
            "value": 124.82754373985655,
            "range": "± 0.20",
            "unit": "ns"
          },
          {
            "name": "temp_file_io/dev_shm_write_read_delete/10MB",
            "value": 9077676.106666666,
            "range": "± 11146.76",
            "unit": "ns"
          },
          {
            "name": "temp_file_io/dev_shm_write_read_delete/1MB",
            "value": 678464.988353057,
            "range": "± 639.90",
            "unit": "ns"
          },
          {
            "name": "temp_file_io/dev_shm_write_read_delete/2.8MB_typical",
            "value": 2463490.7761904765,
            "range": "± 2761.92",
            "unit": "ns"
          },
          {
            "name": "temp_file_io/dev_shm_write_read_delete/5MB",
            "value": 4568754.523636364,
            "range": "± 5534.86",
            "unit": "ns"
          },
          {
            "name": "temp_file_io/memory_copy_baseline/10MB",
            "value": 341270.2734266508,
            "range": "± 779.62",
            "unit": "ns"
          },
          {
            "name": "temp_file_io/memory_copy_baseline/1MB",
            "value": 33975.76095030868,
            "range": "± 59.22",
            "unit": "ns"
          },
          {
            "name": "temp_file_io/memory_copy_baseline/2.8MB_typical",
            "value": 94183.28992374595,
            "range": "± 136.64",
            "unit": "ns"
          },
          {
            "name": "temp_file_io/memory_copy_baseline/5MB",
            "value": 170521.96276455742,
            "range": "± 385.87",
            "unit": "ns"
          },
          {
            "name": "temp_file_io/optimized_temp_path/10MB",
            "value": 9180031.165,
            "range": "± 9808.43",
            "unit": "ns"
          },
          {
            "name": "temp_file_io/optimized_temp_path/1MB",
            "value": 684248.0772111723,
            "range": "± 1078.83",
            "unit": "ns"
          },
          {
            "name": "temp_file_io/optimized_temp_path/2.8MB_typical",
            "value": 2434337.5809523817,
            "range": "± 1474.33",
            "unit": "ns"
          },
          {
            "name": "temp_file_io/optimized_temp_path/5MB",
            "value": 4526061.11090909,
            "range": "± 5444.72",
            "unit": "ns"
          },
          {
            "name": "temp_file_io/system_temp_write_only/10MB",
            "value": 7634217.727142858,
            "range": "± 11219.60",
            "unit": "ns"
          },
          {
            "name": "temp_file_io/system_temp_write_only/1MB",
            "value": 768003.4561558042,
            "range": "± 778.69",
            "unit": "ns"
          },
          {
            "name": "temp_file_io/system_temp_write_only/2.8MB_typical",
            "value": 2069871.9028000005,
            "range": "± 1521.21",
            "unit": "ns"
          },
          {
            "name": "temp_file_io/system_temp_write_only/5MB",
            "value": 3684628.852142856,
            "range": "± 4159.71",
            "unit": "ns"
          },
          {
            "name": "temp_file_io/system_temp_write_read_delete/10MB",
            "value": 8627982.891666662,
            "range": "± 11464.45",
            "unit": "ns"
          },
          {
            "name": "temp_file_io/system_temp_write_read_delete/1MB",
            "value": 836045.9819311987,
            "range": "± 409.29",
            "unit": "ns"
          },
          {
            "name": "temp_file_io/system_temp_write_read_delete/2.8MB_typical",
            "value": 2242979.0830434784,
            "range": "± 1486.21",
            "unit": "ns"
          },
          {
            "name": "temp_file_io/system_temp_write_read_delete/5MB",
            "value": 3995230.5715384595,
            "range": "± 4812.93",
            "unit": "ns"
          },
          {
            "name": "uv_to_speed_direction/100_conversions",
            "value": 1913.1293178451174,
            "range": "± 3.96",
            "unit": "ns"
          },
          {
            "name": "wind_speed_distribution/speed/calm",
            "value": 2411280.0665167193,
            "range": "± 1391.85",
            "unit": "ns"
          },
          {
            "name": "wind_speed_distribution/speed/gale",
            "value": 4327584.373563858,
            "range": "± 4098.82",
            "unit": "ns"
          },
          {
            "name": "wind_speed_distribution/speed/moderate",
            "value": 2965361.76824351,
            "range": "± 2682.91",
            "unit": "ns"
          },
          {
            "name": "wind_speed_distribution/speed/strong",
            "value": 3921257.505878236,
            "range": "± 12732.92",
            "unit": "ns"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "JoegottabeGitenme@users.noreply.github.com",
            "name": "JoegottabeGitenme",
            "username": "JoegottabeGitenme"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "95a4a1ccd1a10d96b46bb2e24ad3b2907d19474d",
          "message": "Merge pull request #6 from JoegottabeGitenme/feature/automated-testing\n\nFeature/automated testing",
          "timestamp": "2025-12-26T08:13:25-07:00",
          "tree_id": "bfb335a7444bc0b750eaddedf80bbbc1cf29a4e7",
          "url": "https://github.com/JoegottabeGitenme/JoeGCServices/commit/95a4a1ccd1a10d96b46bb2e24ad3b2907d19474d"
        },
        "date": 1766762053367,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "barb_full_pipeline/render_and_png",
            "value": 4639217.520479216,
            "range": "± 5952.68",
            "unit": "ns"
          },
          {
            "name": "barb_positions/pixel_grid/tile_1024",
            "value": 1662.6741885617184,
            "range": "± 2.88",
            "unit": "ns"
          },
          {
            "name": "barb_positions/pixel_grid/tile_256",
            "value": 399.0142396403697,
            "range": "± 0.61",
            "unit": "ns"
          },
          {
            "name": "barb_positions/pixel_grid/tile_256_dense",
            "value": 549.7046968716508,
            "range": "± 2.26",
            "unit": "ns"
          },
          {
            "name": "barb_positions/pixel_grid/tile_256_sparse",
            "value": 187.1183752273211,
            "range": "± 0.12",
            "unit": "ns"
          },
          {
            "name": "barb_positions/pixel_grid/tile_512",
            "value": 600.3919216112683,
            "range": "± 1.07",
            "unit": "ns"
          },
          {
            "name": "barb_positions_geographic/geographic/conus_large",
            "value": 2766.4033107902355,
            "range": "± 1.12",
            "unit": "ns"
          },
          {
            "name": "barb_positions_geographic/geographic/conus_z5",
            "value": 2856.456552474101,
            "range": "± 1.87",
            "unit": "ns"
          },
          {
            "name": "barb_positions_geographic/geographic/local_z11",
            "value": 11798.295431455395,
            "range": "± 10.81",
            "unit": "ns"
          },
          {
            "name": "barb_positions_geographic/geographic/region_z8",
            "value": 1920.1330082022894,
            "range": "± 3.17",
            "unit": "ns"
          },
          {
            "name": "barb_size_impact/size/108",
            "value": 1094926.9515797466,
            "range": "± 607.4",
            "unit": "ns"
          },
          {
            "name": "barb_size_impact/size/24",
            "value": 475087.7433587474,
            "range": "± 368.36",
            "unit": "ns"
          },
          {
            "name": "barb_size_impact/size/40",
            "value": 510782.7760195229,
            "range": "± 586.46",
            "unit": "ns"
          },
          {
            "name": "barb_size_impact/size/64",
            "value": 601051.4856122704,
            "range": "± 256.64",
            "unit": "ns"
          },
          {
            "name": "color_functions/pressure_color_100",
            "value": 539.0752762844107,
            "range": "± 0.43",
            "unit": "ns"
          },
          {
            "name": "color_functions/temperature_color_100",
            "value": 724.4830005321907,
            "range": "± 0.38",
            "unit": "ns"
          },
          {
            "name": "color_functions/wind_speed_color_40",
            "value": 199.15253860100023,
            "range": "± 0.12",
            "unit": "ns"
          },
          {
            "name": "connect_segments/smooth/128x128_993seg",
            "value": 545472.9379362835,
            "range": "± 189.1",
            "unit": "ns"
          },
          {
            "name": "connect_segments/smooth/256x256_2017seg",
            "value": 2169745.2679166673,
            "range": "± 606.15",
            "unit": "ns"
          },
          {
            "name": "full_contour_pipeline/contour_256x256_7levels",
            "value": 8765924.519022124,
            "range": "± 5152.19",
            "unit": "ns"
          },
          {
            "name": "full_contour_pipeline/contour_256x256_dense",
            "value": 68974228.85,
            "range": "± 20034.98",
            "unit": "ns"
          },
          {
            "name": "full_pipeline/temperature_tile_256x256",
            "value": 2961243.8364705876,
            "range": "± 935.15",
            "unit": "ns"
          },
          {
            "name": "full_pipeline/temperature_tile_512x512",
            "value": 11777707.933999998,
            "range": "± 3357.97",
            "unit": "ns"
          },
          {
            "name": "generate_all_contours/20_levels/128x128",
            "value": 5696038.0468075,
            "range": "± 3171.12",
            "unit": "ns"
          },
          {
            "name": "generate_all_contours/20_levels/256x256",
            "value": 21607559.295710474,
            "range": "± 8305.14",
            "unit": "ns"
          },
          {
            "name": "generate_all_contours/4_levels/128x128",
            "value": 1169360.4556605457,
            "range": "± 850.6",
            "unit": "ns"
          },
          {
            "name": "generate_all_contours/4_levels/256x256",
            "value": 4432397.226970718,
            "range": "± 1514.78",
            "unit": "ns"
          },
          {
            "name": "generate_contour_levels/levels/0-100_by_10",
            "value": 87.11634818538484,
            "range": "± 0.15",
            "unit": "ns"
          },
          {
            "name": "generate_contour_levels/levels/0-100_by_2",
            "value": 316.13949558755024,
            "range": "± 0.52",
            "unit": "ns"
          },
          {
            "name": "generate_contour_levels/levels/0-100_by_5",
            "value": 171.49758940750283,
            "range": "± 0.28",
            "unit": "ns"
          },
          {
            "name": "generate_contour_levels/levels/neg50-50_by_5",
            "value": 170.04518237985155,
            "range": "± 0.25",
            "unit": "ns"
          },
          {
            "name": "generate_contour_levels/levels/pressure_4hPa",
            "value": 306.73341543213724,
            "range": "± 0.36",
            "unit": "ns"
          },
          {
            "name": "goes_color/ir_enhanced/1024x1024",
            "value": 5698197.87,
            "range": "± 1805.94",
            "unit": "ns"
          },
          {
            "name": "goes_color/ir_enhanced/256x256",
            "value": 330345.2026035377,
            "range": "± 142.52",
            "unit": "ns"
          },
          {
            "name": "goes_color/ir_enhanced/512x512",
            "value": 1428777.543743859,
            "range": "± 981.02",
            "unit": "ns"
          },
          {
            "name": "goes_color/visible_grayscale/1024x1024",
            "value": 9852003.523333333,
            "range": "± 2419.82",
            "unit": "ns"
          },
          {
            "name": "goes_color/visible_grayscale/256x256",
            "value": 613773.2887138363,
            "range": "± 228.73",
            "unit": "ns"
          },
          {
            "name": "goes_color/visible_grayscale/512x512",
            "value": 2466799.723809524,
            "range": "± 586.35",
            "unit": "ns"
          },
          {
            "name": "goes_pipeline/color_and_png_only_256x256",
            "value": 1956481.7516754603,
            "range": "± 1612.14",
            "unit": "ns"
          },
          {
            "name": "goes_pipeline/ir_tile_256x256",
            "value": 15552839.2675,
            "range": "± 6157.56",
            "unit": "ns"
          },
          {
            "name": "goes_pipeline/resample_only_256x256",
            "value": 13568384.2425,
            "range": "± 3147.37",
            "unit": "ns"
          },
          {
            "name": "goes_pipeline/visible_tile_256x256",
            "value": 15156952.015,
            "range": "± 7406.81",
            "unit": "ns"
          },
          {
            "name": "goes_png/encode/256x256",
            "value": 1580693.467893488,
            "range": "± 1145.5",
            "unit": "ns"
          },
          {
            "name": "goes_png/encode/512x512",
            "value": 6463221.2975,
            "range": "± 1817.24",
            "unit": "ns"
          },
          {
            "name": "goes_projection/geo_to_grid/1048576",
            "value": 172213294.39,
            "range": "± 37713.52",
            "unit": "ns"
          },
          {
            "name": "goes_projection/geo_to_grid/262144",
            "value": 43053917.67,
            "range": "± 8540.34",
            "unit": "ns"
          },
          {
            "name": "goes_projection/geo_to_grid/65536",
            "value": 10762607.943999995,
            "range": "± 1624.3",
            "unit": "ns"
          },
          {
            "name": "goes_projection/geo_to_scan/65536",
            "value": 10730754.015999995,
            "range": "± 3374.64",
            "unit": "ns"
          },
          {
            "name": "goes_resample/bilinear_only/central_us_z7",
            "value": 678947.4789308778,
            "range": "± 496.34",
            "unit": "ns"
          },
          {
            "name": "goes_resample/bilinear_only/full_conus_z4",
            "value": 678697.8087989176,
            "range": "± 308.17",
            "unit": "ns"
          },
          {
            "name": "goes_resample/bilinear_only/full_conus_z4_512",
            "value": 2698094.336842105,
            "range": "± 600.5",
            "unit": "ns"
          },
          {
            "name": "goes_resample/bilinear_only/kansas_city_z10",
            "value": 678546.135277414,
            "range": "± 652.04",
            "unit": "ns"
          },
          {
            "name": "goes_resample/with_projection/central_us_z7",
            "value": 13776319.925,
            "range": "± 3205.12",
            "unit": "ns"
          },
          {
            "name": "goes_resample/with_projection/full_conus_z4",
            "value": 13544278.41,
            "range": "± 7367.1",
            "unit": "ns"
          },
          {
            "name": "goes_resample/with_projection/full_conus_z4_512",
            "value": 54471923.79,
            "range": "± 63040.37",
            "unit": "ns"
          },
          {
            "name": "goes_resample/with_projection/kansas_city_z10",
            "value": 13732667.68,
            "range": "± 1596.27",
            "unit": "ns"
          },
          {
            "name": "line_width_impact/width/1",
            "value": 6414883.063922903,
            "range": "± 2409.17",
            "unit": "ns"
          },
          {
            "name": "line_width_impact/width/2",
            "value": 9090137.251469847,
            "range": "± 3609.23",
            "unit": "ns"
          },
          {
            "name": "line_width_impact/width/4",
            "value": 9424378.494647447,
            "range": "± 2351.86",
            "unit": "ns"
          },
          {
            "name": "line_width_impact/width/8",
            "value": 9960270.297749942,
            "range": "± 3361.7",
            "unit": "ns"
          },
          {
            "name": "march_squares/noisy_single_level/128x128",
            "value": 199628.11517290713,
            "range": "± 153.47",
            "unit": "ns"
          },
          {
            "name": "march_squares/noisy_single_level/256x256",
            "value": 785508.1570511982,
            "range": "± 425.13",
            "unit": "ns"
          },
          {
            "name": "march_squares/noisy_single_level/512x512",
            "value": 3170119.94375,
            "range": "± 3885.5",
            "unit": "ns"
          },
          {
            "name": "march_squares/noisy_single_level/64x64",
            "value": 54209.34989645428,
            "range": "± 40.05",
            "unit": "ns"
          },
          {
            "name": "march_squares/smooth_single_level/128x128",
            "value": 157368.56548937646,
            "range": "± 98.92",
            "unit": "ns"
          },
          {
            "name": "march_squares/smooth_single_level/256x256",
            "value": 573599.5881877627,
            "range": "± 300.43",
            "unit": "ns"
          },
          {
            "name": "march_squares/smooth_single_level/512x512",
            "value": 2175903.2134782607,
            "range": "± 407.48",
            "unit": "ns"
          },
          {
            "name": "march_squares/smooth_single_level/64x64",
            "value": 46197.13335708418,
            "range": "± 39.49",
            "unit": "ns"
          },
          {
            "name": "netcdf_io_pattern/current_pattern_with_sync",
            "value": 6878696.56875,
            "range": "± 111252.58",
            "unit": "ns"
          },
          {
            "name": "netcdf_io_pattern/no_sync_pattern",
            "value": 2235353.9065217385,
            "range": "± 1585.98",
            "unit": "ns"
          },
          {
            "name": "netcdf_io_pattern/sequential_3x_operations",
            "value": 6693362.4775,
            "range": "± 3502.72",
            "unit": "ns"
          },
          {
            "name": "png_encoding/create_png/1024x1024",
            "value": 27535957.015,
            "range": "± 22715.07",
            "unit": "ns"
          },
          {
            "name": "png_encoding/create_png/256x256",
            "value": 1667529.9635970218,
            "range": "± 2222.83",
            "unit": "ns"
          },
          {
            "name": "png_encoding/create_png/512x512",
            "value": 6896697.6525,
            "range": "± 2353.68",
            "unit": "ns"
          },
          {
            "name": "projection_lut/compute_lut_z5",
            "value": 13322471.8475,
            "range": "± 1355.62",
            "unit": "ns"
          },
          {
            "name": "projection_lut/compute_lut_z7",
            "value": 14030218.6325,
            "range": "± 1525.53",
            "unit": "ns"
          },
          {
            "name": "projection_lut/on_the_fly/z5_central_conus",
            "value": 14426943.155,
            "range": "± 8910.35",
            "unit": "ns"
          },
          {
            "name": "projection_lut/on_the_fly/z6_midwest",
            "value": 15036832.09,
            "range": "± 1821.4",
            "unit": "ns"
          },
          {
            "name": "projection_lut/on_the_fly/z7_detailed",
            "value": 15046464.255,
            "range": "± 2063.15",
            "unit": "ns"
          },
          {
            "name": "projection_lut/with_lut/z5_central_conus",
            "value": 779463.5035092621,
            "range": "± 676.16",
            "unit": "ns"
          },
          {
            "name": "projection_lut/with_lut/z6_midwest",
            "value": 785889.6076527601,
            "range": "± 355.64",
            "unit": "ns"
          },
          {
            "name": "projection_lut/with_lut/z7_detailed",
            "value": 777055.3484218541,
            "range": "± 560.5",
            "unit": "ns"
          },
          {
            "name": "render_contours_to_canvas/4_levels/256x256",
            "value": 3480188.1736801513,
            "range": "± 1688.97",
            "unit": "ns"
          },
          {
            "name": "render_contours_to_canvas/4_levels/512x512",
            "value": 4528081.4194518,
            "range": "± 6974.33",
            "unit": "ns"
          },
          {
            "name": "render_grid/generic/1024x1024",
            "value": 4533680.235833333,
            "range": "± 799.76",
            "unit": "ns"
          },
          {
            "name": "render_grid/generic/256x256",
            "value": 228001.31405319768,
            "range": "± 173.9",
            "unit": "ns"
          },
          {
            "name": "render_grid/generic/512x512",
            "value": 1077268.935708867,
            "range": "± 1626.22",
            "unit": "ns"
          },
          {
            "name": "render_other/humidity",
            "value": 736190.6506169996,
            "range": "± 748.73",
            "unit": "ns"
          },
          {
            "name": "render_other/pressure",
            "value": 691397.5859716365,
            "range": "± 518",
            "unit": "ns"
          },
          {
            "name": "render_other/wind_speed",
            "value": 556355.1845173545,
            "range": "± 212.88",
            "unit": "ns"
          },
          {
            "name": "render_temperature/celsius/1024x1024",
            "value": 12486728.588000007,
            "range": "± 1730.75",
            "unit": "ns"
          },
          {
            "name": "render_temperature/celsius/256x256",
            "value": 789884.35951045,
            "range": "± 291.77",
            "unit": "ns"
          },
          {
            "name": "render_temperature/celsius/512x512",
            "value": 3141174.363125,
            "range": "± 1496.37",
            "unit": "ns"
          },
          {
            "name": "render_wind_barbs/uniform_wind/tile_256_default",
            "value": 473172.01640795905,
            "range": "± 279.11",
            "unit": "ns"
          },
          {
            "name": "render_wind_barbs/uniform_wind/tile_256_dense",
            "value": 1516495.7490822368,
            "range": "± 6711.44",
            "unit": "ns"
          },
          {
            "name": "render_wind_barbs/uniform_wind/tile_512",
            "value": 1897949.135934192,
            "range": "± 1380.96",
            "unit": "ns"
          },
          {
            "name": "render_wind_barbs/varied_wind_256",
            "value": 4114191.8317579753,
            "range": "± 3431.54",
            "unit": "ns"
          },
          {
            "name": "render_wind_barbs_aligned/aligned/bbox_10deg",
            "value": 3498192.7250817665,
            "range": "± 11260.53",
            "unit": "ns"
          },
          {
            "name": "render_wind_barbs_aligned/aligned/bbox_3deg",
            "value": 2779166.5201905826,
            "range": "± 2235.09",
            "unit": "ns"
          },
          {
            "name": "render_wind_barbs_aligned/aligned/bbox_conus",
            "value": 1396122.9589164557,
            "range": "± 1557.27",
            "unit": "ns"
          },
          {
            "name": "resample_grid/GFS_to_large/bilinear",
            "value": 2144358.91375,
            "range": "± 808.11",
            "unit": "ns"
          },
          {
            "name": "resample_grid/GFS_to_tile/bilinear",
            "value": 544148.9072326731,
            "range": "± 286.63",
            "unit": "ns"
          },
          {
            "name": "resample_grid/GOES_to_tile/bilinear",
            "value": 531427.1006905976,
            "range": "± 308.43",
            "unit": "ns"
          },
          {
            "name": "resample_grid/MRMS_to_tile/bilinear",
            "value": 566556.3329640214,
            "range": "± 295.78",
            "unit": "ns"
          },
          {
            "name": "resample_grid/downscale_2x/bilinear",
            "value": 539271.8463956423,
            "range": "± 603.62",
            "unit": "ns"
          },
          {
            "name": "resample_grid/upscale_2x/bilinear",
            "value": 2135894.2908333335,
            "range": "± 331.23",
            "unit": "ns"
          },
          {
            "name": "smooth_contour/1_passes/100_points",
            "value": 444.1947687284956,
            "range": "± 0.82",
            "unit": "ns"
          },
          {
            "name": "smooth_contour/1_passes/10_points",
            "value": 67.62366149336798,
            "range": "± 0.09",
            "unit": "ns"
          },
          {
            "name": "smooth_contour/1_passes/500_points",
            "value": 2048.633061247985,
            "range": "± 3.63",
            "unit": "ns"
          },
          {
            "name": "smooth_contour/1_passes/50_points",
            "value": 226.1455916913905,
            "range": "± 0.17",
            "unit": "ns"
          },
          {
            "name": "smooth_contour/2_passes/100_points",
            "value": 1256.4491213830029,
            "range": "± 1.08",
            "unit": "ns"
          },
          {
            "name": "smooth_contour/2_passes/10_points",
            "value": 155.69873311410944,
            "range": "± 0.12",
            "unit": "ns"
          },
          {
            "name": "smooth_contour/2_passes/500_points",
            "value": 5875.42809889044,
            "range": "± 9.46",
            "unit": "ns"
          },
          {
            "name": "smooth_contour/2_passes/50_points",
            "value": 654.701865547575,
            "range": "± 0.45",
            "unit": "ns"
          },
          {
            "name": "smooth_contour/3_passes/100_points",
            "value": 2816.504375797505,
            "range": "± 2.3",
            "unit": "ns"
          },
          {
            "name": "smooth_contour/3_passes/10_points",
            "value": 317.95634648434674,
            "range": "± 0.42",
            "unit": "ns"
          },
          {
            "name": "smooth_contour/3_passes/500_points",
            "value": 13580.669253202203,
            "range": "± 17.44",
            "unit": "ns"
          },
          {
            "name": "smooth_contour/3_passes/50_points",
            "value": 1469.1703597980074,
            "range": "± 4.57",
            "unit": "ns"
          },
          {
            "name": "subset_grid/americas/GFS",
            "value": 317545.2412487296,
            "range": "± 222.61",
            "unit": "ns"
          },
          {
            "name": "subset_grid/conus/GFS",
            "value": 53975.467333825545,
            "range": "± 47.75",
            "unit": "ns"
          },
          {
            "name": "subset_grid/europe/GFS",
            "value": 44002.000574326696,
            "range": "± 48.84",
            "unit": "ns"
          },
          {
            "name": "subset_grid/small_region/GFS",
            "value": 124.82754373985655,
            "range": "± 0.2",
            "unit": "ns"
          },
          {
            "name": "temp_file_io/dev_shm_write_read_delete/10MB",
            "value": 9077676.106666666,
            "range": "± 11146.76",
            "unit": "ns"
          },
          {
            "name": "temp_file_io/dev_shm_write_read_delete/1MB",
            "value": 678464.988353057,
            "range": "± 639.9",
            "unit": "ns"
          },
          {
            "name": "temp_file_io/dev_shm_write_read_delete/2.8MB_typical",
            "value": 2463490.7761904765,
            "range": "± 2761.92",
            "unit": "ns"
          },
          {
            "name": "temp_file_io/dev_shm_write_read_delete/5MB",
            "value": 4568754.523636364,
            "range": "± 5534.86",
            "unit": "ns"
          },
          {
            "name": "temp_file_io/memory_copy_baseline/10MB",
            "value": 341270.2734266508,
            "range": "± 779.62",
            "unit": "ns"
          },
          {
            "name": "temp_file_io/memory_copy_baseline/1MB",
            "value": 33975.76095030868,
            "range": "± 59.22",
            "unit": "ns"
          },
          {
            "name": "temp_file_io/memory_copy_baseline/2.8MB_typical",
            "value": 94183.28992374595,
            "range": "± 136.64",
            "unit": "ns"
          },
          {
            "name": "temp_file_io/memory_copy_baseline/5MB",
            "value": 170521.96276455742,
            "range": "± 385.87",
            "unit": "ns"
          },
          {
            "name": "temp_file_io/optimized_temp_path/10MB",
            "value": 9180031.165,
            "range": "± 9808.43",
            "unit": "ns"
          },
          {
            "name": "temp_file_io/optimized_temp_path/1MB",
            "value": 684248.0772111723,
            "range": "± 1078.83",
            "unit": "ns"
          },
          {
            "name": "temp_file_io/optimized_temp_path/2.8MB_typical",
            "value": 2434337.5809523817,
            "range": "± 1474.33",
            "unit": "ns"
          },
          {
            "name": "temp_file_io/optimized_temp_path/5MB",
            "value": 4526061.11090909,
            "range": "± 5444.72",
            "unit": "ns"
          },
          {
            "name": "temp_file_io/system_temp_write_only/10MB",
            "value": 7634217.727142858,
            "range": "± 11219.6",
            "unit": "ns"
          },
          {
            "name": "temp_file_io/system_temp_write_only/1MB",
            "value": 768003.4561558042,
            "range": "± 778.69",
            "unit": "ns"
          },
          {
            "name": "temp_file_io/system_temp_write_only/2.8MB_typical",
            "value": 2069871.9028000005,
            "range": "± 1521.21",
            "unit": "ns"
          },
          {
            "name": "temp_file_io/system_temp_write_only/5MB",
            "value": 3684628.852142856,
            "range": "± 4159.71",
            "unit": "ns"
          },
          {
            "name": "temp_file_io/system_temp_write_read_delete/10MB",
            "value": 8627982.891666662,
            "range": "± 11464.45",
            "unit": "ns"
          },
          {
            "name": "temp_file_io/system_temp_write_read_delete/1MB",
            "value": 836045.9819311987,
            "range": "± 409.29",
            "unit": "ns"
          },
          {
            "name": "temp_file_io/system_temp_write_read_delete/2.8MB_typical",
            "value": 2242979.0830434784,
            "range": "± 1486.21",
            "unit": "ns"
          },
          {
            "name": "temp_file_io/system_temp_write_read_delete/5MB",
            "value": 3995230.5715384595,
            "range": "± 4812.93",
            "unit": "ns"
          },
          {
            "name": "uv_to_speed_direction/100_conversions",
            "value": 1913.1293178451174,
            "range": "± 3.96",
            "unit": "ns"
          },
          {
            "name": "wind_speed_distribution/speed/calm",
            "value": 2411280.0665167193,
            "range": "± 1391.85",
            "unit": "ns"
          },
          {
            "name": "wind_speed_distribution/speed/gale",
            "value": 4327584.373563858,
            "range": "± 4098.82",
            "unit": "ns"
          },
          {
            "name": "wind_speed_distribution/speed/moderate",
            "value": 2965361.76824351,
            "range": "± 2682.91",
            "unit": "ns"
          },
          {
            "name": "wind_speed_distribution/speed/strong",
            "value": 3921257.505878236,
            "range": "± 12732.92",
            "unit": "ns"
          }
        ]
      }
    ]
  }
}