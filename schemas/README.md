# JSON Schemas for Configuration Files

This directory contains JSON schemas for validating YAML configuration files in this project.

## Schemas

| Schema | Purpose | Config Files |
|--------|---------|--------------|
| `weather-model.schema.json` | Weather data model definitions | `config/models/*.yaml` |
| `wms-layer.schema.json` | WMS/WMTS layer definitions | `config/layers/*.yaml` |
| `edr-collection.schema.json` | OGC EDR API collections | `config/edr/*.yaml` (excl. locations.yaml) |
| `edr-locations.schema.json` | Named EDR locations | `config/edr/locations.yaml` |
| `ingestion.schema.json` | Global ingestion settings | `config/ingestion.yaml` |
| `load-test-scenario.schema.json` | WMS/WMTS load test scenarios | `validation/load-test/scenarios/*.yaml` |
| `edr-load-test-scenario.schema.json` | EDR API load test scenarios | `validation/load-test/scenarios/*-edr.yaml` |

## Validation

Run the validation script to check all config files against their schemas:

```bash
# Validate all configs
./scripts/validate-configs.sh

# Validate with detailed error output
./scripts/validate-configs.sh --verbose
```

The script requires Node.js and will automatically install required dependencies (`ajv`, `js-yaml`) in a temporary directory.

## Editor Integration

### VS Code

1. Install the [YAML extension](https://marketplace.visualstudio.com/items?itemName=redhat.vscode-yaml)
2. Add to your `.vscode/settings.json`:

```json
{
  "yaml.schemas": {
    "./schemas/weather-model.schema.json": ["config/models/*.yaml"],
    "./schemas/wms-layer.schema.json": ["config/layers/*.yaml"],
    "./schemas/edr-collection.schema.json": [
      "config/edr/hrrr.yaml",
      "config/edr/gfs.yaml",
      "config/edr/mrms.yaml",
      "config/edr/goes*.yaml"
    ],
    "./schemas/edr-locations.schema.json": ["config/edr/locations.yaml"],
    "./schemas/ingestion.schema.json": ["config/ingestion.yaml"],
    "./schemas/load-test-scenario.schema.json": [
      "validation/load-test/scenarios/hrrr.yaml",
      "validation/load-test/scenarios/gfs.yaml",
      "validation/load-test/scenarios/mrms.yaml",
      "validation/load-test/scenarios/goes.yaml",
      "validation/load-test/scenarios/mixed.yaml"
    ],
    "./schemas/edr-load-test-scenario.schema.json": [
      "validation/load-test/scenarios/*-edr.yaml"
    ]
  }
}
```

### JetBrains (IntelliJ IDEA, WebStorm, etc.)

1. Go to **Settings/Preferences > Languages & Frameworks > Schemas and DTDs > JSON Schema Mappings**
2. Add mappings for each schema:
   - Name: `Weather Model Schema`
   - Schema file: `schemas/weather-model.schema.json`
   - File path pattern: `config/models/*.yaml`
3. Repeat for each schema/pattern pair

Alternatively, create `.idea/jsonSchemas.xml` with the mappings (see project for example).

## Schema Features

- **Permissive**: Schemas use `additionalProperties: true` to allow experimentation
- **Documented**: All properties include descriptions that appear in editor tooltips
- **Type-safe**: Validates data types, enums, and required fields
- **Editor-friendly**: Enables autocomplete and hover documentation

## Adding New Config Types

1. Create a new schema file in `schemas/`
2. Follow the existing schema patterns (use draft-07, add `$id`, set `additionalProperties: true`)
3. Update `scripts/validate-configs.sh` to validate the new files
4. Update your editor settings to associate the schema with the config files
