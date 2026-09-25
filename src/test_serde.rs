use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "cmd", content = "payload")]
pub enum ShowCommandTagged {
    GoLive { item_index: Option<usize>, slide_index: Option<usize> },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ShowCommandExternal {
    GoLive { item_index: Option<usize>, slide_index: Option<usize> },
}

fn main() {
    let json_str = r#"{"GoLive": {"item_index": null, "slide_index": 0}}"#;
    
    let tagged: Result<ShowCommandTagged, _> = serde_json::from_str(json_str);
    println!("Tagged: {:?}", tagged);
    
    let external: Result<ShowCommandExternal, _> = serde_json::from_str(json_str);
    println!("External: {:?}", external);
}
