# 铜层连通谱：Gerber 图层、孔表与网络候选核验台（离线案例包）

本案例包专门面向 PCB 制程团队的工作链：导入 Gerber / Excellon / 层叠说明，
重建铜区、孔、阻焊开窗与电气连通候选。所有材料均为领域文件，不是通用表格。

## 材料清单（source/）

| 文件 | 类型 | 单位 / 坐标格式 | 覆盖情形 |
| --- | --- | --- | --- |
| `top.gbr` | Gerber | inch，`2:5`，前导零抑制 | 单位切换、负（clear）极性按序叠加、aperture 多尺寸、边界相切 |
| `inner.gbr` | Gerber | mm，`3:3` | 与 inch 层混合装配、单位换算 |
| `bottom.gbr` | Gerber | inch，`2:5` | 对侧铜层 |
| `sm_top.gbr` | Gerber | inch，`2:5` | 阻焊开窗 |
| `extra.gbr` | Gerber | mm，`3:3` | 初始 `unassigned`，后经“层角色修订”成为内层铜 |
| `plated.drl` | Excellon | mm，`2:5` | PTH 通孔、盲孔 `span=0:1`、埋孔 `span=2` |
| `nonplated.drl` | Excellon | inch，`2:5` | NPTH 落在铜上也不得导通 |
| `stackup.csv` | 层叠 | 行级手工映射 | 图层角色 + 手工映射原样保留 |
| `step_repeat.gbr` | Gerber | inch，`2:5` | `SR` step-repeat 展开 |
| `negative_order.gbr` | Gerber | mm，`3:3` | 正-负-正按操作顺序叠加 |
| `probe_corrupt.gbr` | Gerber | - | 格式损坏反例 |
| `probe_semantic.gbr` | Gerber | - | 语义无解反例（缺单位） |

## 重放叙事（与应用内事件流一致）

1. 基线导入：全部文件的坐标格式、单位、极性、aperture、step-repeat、
   图层角色与手工映射逐条保留。
2. 参数修订：几何接触容差 `0.05mm -> 0.08mm`（新规则版本）。
3. 局部源数据改动：仅替换 `step_repeat.gbr` 的重复次数，影响必须只波及
   该材料的派生对象。
4. 互相冲突的人工决定：对同一条容差候选边分别接受（主分支）与
   否定（分支），矛盾位置在复核界面标出。
5. 层角色修订：`extra.gbr` 从 `unassigned` 改为 `copper_inner`，
   盲埋孔 `span=2` 的语义随之落到新的铜层。

关键判断依据：

- 正负极性按**操作顺序**叠加（不是集合异或）。
- 几何接触容差是**版本参数**：`gap == 0` 为硬接触；`0 < gap <= tol`
  只产生候选边；`gap > tol` 仅为邻近提示，永远不导通。
- 无 netlist 时只给连通**候选**，无法唯一决定时保留全部合法候选解。
