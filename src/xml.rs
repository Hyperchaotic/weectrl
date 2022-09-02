pub use crate::error::Error;

use serde::Deserialize;
use serde_xml_rs;

#[derive(Debug, Deserialize, Clone)]
#[serde(rename = "service")]
pub struct Service {
    #[serde(rename = "serviceType")]
    pub service_type: String,
    #[serde(rename = "serviceId")]
    pub service_id: String,
    #[serde(rename = "controlURL")]
    pub control_url: String,
    #[serde(rename = "eventSubURL")]
    pub event_sub_url: String,
    #[serde(rename = "SCPDURL")]
    pub scp_url: String,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename = "icon")]
pub struct Icon {
    pub mimetype: String, // >jpg</mimetype>
    pub width: String,    // >100</width>
    pub height: String,   // >100</height>
    pub depth: String,    // >100</depth>
    pub url: String,      // >icon.jpg</url>
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename = "iconList")]
pub struct IconList {
    pub icon: Vec<Icon>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename = "serviceList")]
pub struct ServiceList {
    pub service: Vec<Service>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename = "device")]
pub struct Device {
    #[serde(rename = "deviceType")]
    pub device_type: String, // >urn:Belkin:device:lightswitch:1</deviceType>
    #[serde(rename = "friendlyName")]
    pub friendly_name: String, // >Kitchen light</friendlyName>
    pub manufacturer: String, // >Belkin International Inc.</manufacturer>
    #[serde(rename = "manufacturerURL")]
    pub manufacturer_url: String, // >http://www.belkin.com</manufacturerURL>
    #[serde(rename = "modelDescription")]
    pub model_description: String, // >Belkin Plugin Socket 1.0</modelDescription>
    #[serde(rename = "modelName")]
    pub model_name: String, // >LightSwitch</modelName>
    #[serde(rename = "modelNumber")]
    pub model_number: String, // >1.0</modelNumber>
    #[serde(rename = "modelURL")]
    pub model_url: String, // >http://www.belkin.com/plugin/</modelURL>
    #[serde(rename = "serialNumber")]
    pub serial_number: String, // >221435K13005D9</serialNumber>
    #[serde(rename = "UDN")]
    pub udn: String, // >uuid:Lightswitch-1_0-221435K13005D9</UDN>
    #[serde(rename = "UPC")]
    pub upc: String, // >123456789</UPC>
    #[serde(rename = "macAddress")]
    pub mac_address: String, // >94103E4830D0</macAddress>
    #[serde(rename = "firmwareVersion")]
    pub firmware_version: String, // >WeMo_WW_2.00.10937.PVT-OWRT-LS</firmwareVersion>
    #[serde(rename = "iconVersion")]
    pub icon_version: String, // >0|49153</iconVersion>
    #[serde(rename = "binaryState")]
    pub binary_state: String, // >0</binaryState>
    #[serde(rename = "iconList")]
    pub icon_list: IconList,
    #[serde(rename = "serviceList")]
    pub service_list: ServiceList,
    #[serde(rename = "presentationURL")]
    pub presentation_url: String,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename = "specVersion")]
pub struct SpecVersion {
    pub major: String,
    pub minor: String,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename = "root")]
pub struct Root {
    #[serde(rename = "specVersion")]
    pub spec_version: SpecVersion,
    pub device: Device,
}

pub fn parse_services(data: &str) -> Result<Root, Error> {
    let r: Root = serde_xml_rs::de::from_str(data)?;
    Ok(r)
}

pub fn get_binary_state(data: &str) -> Option<u8> {
    if data.contains("<BinaryState>1</BinaryState>") {
        return Some(1);
    } else if data.contains("<BinaryState>0</BinaryState>") {
        return Some(0);
    }
    None
}

pub const SETBINARYSTATEOFF: &'static str = "<?xml version=\"1.0\" encoding=\"utf-8\"?>
<s:Envelope \
     xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" \
     s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\">
  <s:Body>
    \
     <u:SetBinaryState xmlns:u=\"urn:Belkin:service:basicevent:1\">
      \
     <BinaryState>0</BinaryState>
    </u:SetBinaryState>
  </s:Body>
</s:Envelope>";

pub const SETBINARYSTATEON: &'static str = "<?xml version=\"1.0\" encoding=\"utf-8\"?>
<s:Envelope \
     xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" \
     s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\">
  <s:Body>
    \
     <u:SetBinaryState xmlns:u=\"urn:Belkin:service:basicevent:1\">
      \
     <BinaryState>1</BinaryState>
    </u:SetBinaryState>
  </s:Body>
</s:Envelope>";

pub const GETBINARYSTATE: &'static str = "
<?xml version=\"1.0\" encoding=\"utf-8\"?>
<s:Envelope \
     xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" \
     s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\">
    <s:Body>
     <u:GetBinaryState xmlns:u=\"urn:Belkin:service:basicevent:1\">
     <BinaryState>1</BinaryState>
        </u:GetBinaryState>
    </s:Body>
</s:Envelope>";
