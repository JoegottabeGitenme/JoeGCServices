//! WMS GetCapabilities client for querying layer metadata.

use anyhow::{anyhow, Result};
use quick_xml::events::Event;
use quick_xml::Reader;

/// Query available times for a layer from WMS GetCapabilities.
pub async fn query_layer_times(base_url: &str, layer_name: &str) -> Result<Vec<String>> {
    let url = format!("{}/wms?SERVICE=WMS&REQUEST=GetCapabilities", base_url);
    
    // Fetch GetCapabilities XML
    let response = reqwest::get(&url).await?;
    let xml = response.text().await?;
    
    // Parse times from the XML
    let times = parse_time_dimension(&xml, layer_name)?;
    
    if times.is_empty() {
        return Err(anyhow!(
            "No TIME dimension found for layer '{}'", 
            layer_name
        ));
    }
    
    Ok(times)
}

/// Parse TIME dimension from WMS GetCapabilities XML using proper XML parser.
fn parse_time_dimension(xml: &str, layer_name: &str) -> Result<Vec<String>> {
    let mut reader = Reader::from_str(xml);
    reader.trim_text(true);
    
    let mut buf = Vec::new();
    let mut in_target_layer = false;
    let mut in_dimension = false;
    let mut dimension_content = String::new();
    
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                match e.name().as_ref() {
                    b"Layer" => {
                        // Check if this layer contains our target
                        // We'll track this by watching for the Name tag
                    }
                    b"Name" if !in_target_layer => {
                        // Read the layer name
                        if let Ok(Event::Text(t)) = reader.read_event_into(&mut buf) {
                            if t.unescape()?.as_ref() == layer_name {
                                in_target_layer = true;
                            }
                        }
                    }
                    b"Dimension" if in_target_layer => {
                        // Check if this is the TIME dimension
                        for attr in e.attributes() {
                            if let Ok(attr) = attr {
                                if attr.key.as_ref() == b"name" {
                                    let name = String::from_utf8_lossy(&attr.value);
                                    if name == "TIME" {
                                        in_dimension = true;
                                        dimension_content.clear();
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(t)) if in_dimension => {
                dimension_content.push_str(&t.unescape()?);
            }
            Ok(Event::End(e)) => {
                match e.name().as_ref() {
                    b"Layer" if in_target_layer => {
                        // If we've found the TIME dimension, return it
                        if !dimension_content.is_empty() {
                            let times: Vec<String> = dimension_content
                                .split(',')
                                .map(|s| s.trim().to_string())
                                .filter(|s| !s.is_empty())
                                .collect();
                            return Ok(times);
                        }
                        // Otherwise, this layer doesn't have a TIME dimension
                        in_target_layer = false;
                    }
                    b"Dimension" if in_dimension => {
                        in_dimension = false;
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(anyhow!("XML parsing error at position {}: {:?}", reader.buffer_position(), e)),
            _ => {}
        }
        buf.clear();
    }
    
    Err(anyhow!("No TIME dimension found for layer '{}'", layer_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_time_dimension() {
        let xml = r#"
<Layer>
    <Name>goes18_CMI_C13</Name>
    <Title>GOES18 - Clean IR (10.3µm)</Title>
    <Dimension name="TIME" units="ISO8601" default="2025-12-02T19:21:00Z">2025-12-02T19:21:00Z,2025-12-02T19:16:00Z,2025-12-02T19:11:00Z</Dimension>
</Layer>
        "#;
        
        let times = parse_time_dimension(xml, "goes18_CMI_C13").unwrap();
        assert_eq!(times.len(), 3);
        assert_eq!(times[0], "2025-12-02T19:21:00Z");
        assert_eq!(times[1], "2025-12-02T19:16:00Z");
        assert_eq!(times[2], "2025-12-02T19:11:00Z");
    }
}
