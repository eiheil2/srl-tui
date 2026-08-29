# SRL 使用教程

> SRL（Spaced Repetition Learning）是一个终端里的间隔重复记忆卡应用：
> 纯键盘操作、零依赖、单文件二进制，适合低配设备（如 ARM 小板）日常刷卡。
> 本教程覆盖安装、导入卡组、日常学习、设置与备份。

## 1. 安装

### 方式一：下载现成二进制（推荐 ARM 板）

到 [Releases](https://github.com/eiheil2/srl-tui/releases) 下载
`srl-vX.Y.Z-aarch64-linux-gnu.tar.gz`，解压后：

```bash
tar xzf srl-*-aarch64-linux-gnu.tar.gz
chmod +x srl
./srl --help          # 能打印帮助即成功
```

首次运行前建议装中文字体（否则中文显示为方块）：

```bash
sudo apt install fonts-noto-cjk
```

### 方式二：源码编译

```bash
git clone https://github.com/eiheil2/srl-tui.git
cd srl-tui
cargo install --path .    # 安装到 ~/.cargo/bin
```

## 2. 首次启动

直接运行 `./srl` 会进入卡组列表。全新环境会自动装一个
"Development Workflow" 演示卡组供上手，可以随时删掉（选中后按两次 `d`）。

## 3. 导入卡组

### CSV（自制词表）

两列或三列（第三列为标签，可省略），支持引号包裹的逗号字段：

```csv
front,back,tags
potential,/pəˈtenʃl/ n. 潜力,高频
```

```bash
./srl --import vocab.csv --import-name "牌组名"
./srl --import-folder ./decks/        # 批量导入整个目录（文件名即牌组名）
./srl --list                          # 查看已导入的牌组
```

### Anki 牌组（.apkg / 导出文本）

```bash
./srl --import-anki deck.apkg             # 保留复习进度（到期日/间隔/难度）
./srl --import-anki vocab.txt --import-anki-name "生词"   # Anki 制表符导出
```

支持普通问答卡、多字段模板（取第一个非空字段作背面）和
**完形填空（cloze）卡**：正面挖空为 `＿＿＿`（有提示时显示 `［提示］`），
背面为"答案：…"。多模板笔记只导入第一模板。

### 备份恢复

```bash
./srl --import-backup backup.json      # 恢复（同 id 牌组自动跳过）
./srl --export-backup all.json         # 全量导出
```

## 4. 日常学习

启动后就是**卡组列表**，底部有按键提示：

| 按键 | 作用 |
|------|------|
| `j/k` 或方向键 | 上下选择 |
| `Enter` | 进入学习 |
| `b` | 浏览该牌组的卡片 |
| `n` / `r` | 新建牌组 / 重命名（弹窗输入） |
| `i` | 导入备份（输入路径） |
| `d d` | 删除牌组（按两次确认） |
| `+` / `-` | 每次学习引入的新卡数 ±5 |
| `x` | 导出全部牌组备份 |
| `s` | 统计面板 |
| `t` | 切换主题（10 套配色，持久保存） |
| `q` | 退出 |

### 学习界面

- 顶部是牌组名和会话进度条（`▰▰▰▱▱ 3/12`），下面一行是
  New / Learning / Due / Total 统计。
- 按 **空格** 显示答案（再按翻回正面），随后用 **`1`-`4`** 评分：

| 键 | 评价 | 效果 |
|----|------|------|
| `1` | Again | 完全忘记：新卡 10 分钟后重来，旧卡间隔重置 |
| `2` | Hard | 很吃力：间隔 ×1.2 |
| `3` | Good | 想起来了：间隔 × 难度系数 |
| `4` | Easy | 秒答：间隔 × 系数 × 1.3 |

  每个按键下方会实时预览这次评分的下一次间隔。
- 评分即保存；答错的卡本轮稍后会再出现。
- 学习中按 `a` 加卡、`b` 进浏览器、`Esc` 返回列表。
- 队列清空后出现完成页（本张数 + 用时）。

### 每次学多少？

每次会话默认引入 **20 张新卡**（已到期复习卡不受限、全部出现）。
在卡组列表按 `+`/`-` 调整，或改配置文件里的 `new_per_session`。

## 5. 卡片浏览器

列表页按 `b` 进入：

| 按键 | 作用 |
|------|------|
| `j/k` | 选择卡片（右侧显示正反面与调度信息） |
| `e` | 编辑卡片（Tab 切换正反面，Enter 保存，Esc 取消） |
| `d d` | 删除卡片（两次确认） |
| `a` | 加新卡 |

## 6. 统计

列表页按 `s`：总卡数、总复习数、每日/每周连续打卡、按难度分布
（New / Easy / Good / Hard / Struggling）。

## 7. 配置

配置在 `~/.config/flashcards/config.toml`（Windows 为
`%APPDATA%\flashcards\config.toml`），一般不用手改：

```toml
theme = "default"        # 主题名，按 t 循环
new_per_session = 20     # 每次会话新卡上限
```

牌组数据存放在 `~/.local/share/flashcards/decks/`
（Windows 为 `%LOCALAPPDATA%\flashcards\decks`），
每个牌组一个 JSON 文件；也可用 `--decks-dir` 指定任意目录（便携使用）。

## 8. 数据安全

- 牌组保存是原子写入（临时文件 + 改名），意外断电不会写坏数据。
- 建议定期按 `x`（或 `--export-backup`）导出 JSON 备份；
  恢复用 `i` 或 `--import-backup`，同 id 牌组自动跳过、不会重复。
- 与 Anki 双向互通：`--export-anki out.apkg` 可把全部牌组带回 Anki。

## 9. 常见问题

- **中文显示方块** → 装字体：`sudo apt install fonts-noto-cjk`，重开终端。
- **界面错乱/按键无效** → 终端窗口太小；至少需要约 20 列 × 8 行，
  建议全屏使用。
- **误删牌组** → 删除需要按两次 `d`；真删错了就从备份 `i` 恢复。
- **想临时用另一套牌组** → `./srl --decks-dir /path/to/dir`。
