//! UPnP device description XML (`LOCATION` of an SSDP response).

use crate::transport::{http_get, net_err};
use capture_core::CaptureResult;
use std::net::IpAddr;
use std::time::Duration;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceDescription {
    pub friendly_name: Option<String>,
    pub manufacturer: Option<String>,
    pub model_name: Option<String>,
    pub model_number: Option<String>,
    pub udn: Option<String>,
    pub device_type: Option<String>,
    /// `serviceType` of every service of the root device and embedded devices.
    pub service_types: Vec<String>,
}

fn text_of(node: roxmltree::Node<'_, '_>, name: &str) -> Option<String> {
    node.children()
        .find(|c| c.is_element() && c.tag_name().name() == name)
        .and_then(|c| c.text())
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

/// Parses the root `<device>` of a description document. DTDs are tolerated (some TVs send one)
/// but the node count is capped so a hostile device cannot exhaust memory.
pub fn parse(xml: &str) -> Option<DeviceDescription> {
    let xml = xml.trim_start_matches('\u{feff}');
    let opts = roxmltree::ParsingOptions { allow_dtd: true, nodes_limit: 50_000, ..Default::default() };
    let doc = roxmltree::Document::parse_with_options(xml, opts).ok()?;
    let device = doc.descendants().find(|n| n.is_element() && n.tag_name().name() == "device")?;
    let service_types = doc
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "serviceType")
        .filter_map(|n| n.text().map(|t| t.trim().to_string()))
        .filter(|t| !t.is_empty())
        .collect();
    Some(DeviceDescription {
        friendly_name: text_of(device, "friendlyName"),
        manufacturer: text_of(device, "manufacturer"),
        model_name: text_of(device, "modelName"),
        model_number: text_of(device, "modelNumber"),
        udn: text_of(device, "UDN"),
        device_type: text_of(device, "deviceType"),
        service_types,
    })
}

/// Fetches and parses a description. The URL must point at the device that answered the
/// search (`expected_ip`), so a hostile responder cannot make us fetch arbitrary hosts.
pub fn fetch(location: &str, expected_ip: IpAddr, timeout: Duration) -> CaptureResult<DeviceDescription> {
    let resp = http_get(location, Some(expected_ip), None, timeout, 512 * 1024)?;
    if resp.status != 200 {
        return Err(net_err(format!("{location}: HTTP {}", resp.status)));
    }
    let text = String::from_utf8_lossy(&resp.body);
    parse(&text).ok_or_else(|| net_err(format!("{location}: not a UPnP device description")))
}

#[cfg(test)]
mod tests {
    use super::*;

    pub const LG_XML: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<root xmlns="urn:schemas-upnp-org:device-1-0" xmlns:dlna="urn:schemas-dlna-org:device-1-0">
  <specVersion><major>1</major><minor>0</minor></specVersion>
  <device>
    <deviceType>urn:schemas-upnp-org:device:Basic:1</deviceType>
    <friendlyName>[LG] webOS TV OLED55C1PUB</friendlyName>
    <manufacturer>LG Electronics</manufacturer>
    <modelName>OLED55C1PUB</modelName>
    <modelNumber>55C1</modelNumber>
    <UDN>uuid:0a7a1e0d-1111-2222-3333-cc2d8c9e0d5e</UDN>
    <serviceList>
      <service><serviceType>urn:lge-com:service:webos-second-screen:1</serviceType><serviceId>urn:lge-com:serviceId:webos-second-screen-3000-3001</serviceId></service>
    </serviceList>
  </device>
</root>"#;

    #[test]
    fn parses_lg() {
        let d = parse(LG_XML).unwrap();
        assert_eq!(d.friendly_name.as_deref(), Some("[LG] webOS TV OLED55C1PUB"));
        assert_eq!(d.manufacturer.as_deref(), Some("LG Electronics"));
        assert_eq!(d.model_name.as_deref(), Some("OLED55C1PUB"));
        assert_eq!(d.model_number.as_deref(), Some("55C1"));
        assert_eq!(d.service_types, vec!["urn:lge-com:service:webos-second-screen:1".to_string()]);
    }

    #[test]
    fn tolerates_bom_doctype_and_rejects_garbage() {
        let xml = "\u{feff}<?xml version=\"1.0\"?><!DOCTYPE root><root><device><friendlyName> x </friendlyName></device></root>";
        assert_eq!(parse(xml).unwrap().friendly_name.as_deref(), Some("x"));
        assert!(parse("<html>no device</html>").is_none());
        assert!(parse("not xml").is_none());
    }
}
