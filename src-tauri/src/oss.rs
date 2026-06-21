// OSS client stub
pub struct OssClient { pub region: String, pub bucket: String }
impl OssClient { pub fn new(r: &str, b: &str) -> Self { Self{region:r.into(), bucket:b.into()} } }
