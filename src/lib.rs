//! PCB / Gerber / 几何连通核验台
//! 铜层连通谱:Gerber 图层、孔表与网络候选核验台。
//! 业务规则全部在服务端与存储层,可直接被测试调用。

pub mod model;
pub mod geom;
pub mod parse;
pub mod engine;
pub mod store;
pub mod casepack;
pub mod server;
