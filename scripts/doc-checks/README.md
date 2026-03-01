# 文档质量检查脚本

本目录包含用于维护 claw-kernel 项目文档质量的 CI 检查脚本。

## 📁 文件结构

```
scripts/doc-checks/
├── README.md                    # 本文档
├── check_metadata.py            # 元数据完整性检查
├── check_internal_links.py      # 内部链接检查
├── check_terminology.py         # 术语一致性检查
├── check_versions.py            # 版本号同步检查
├── check_bilingual.py           # 双语同步检查
├── generate_report.py           # 综合报告生成
├── update_timestamps.py         # 文档时间戳更新
├── version-config.yaml          # 版本号配置
└── .markdown-link-check.json    # 外部链接检查配置
```

## 🔧 安装依赖

大部分脚本仅依赖 Python 标准库。可选安装 `pyyaml` 以获得更好的 YAML 解析：

```bash
pip install pyyaml
```

对于外部链接检查，需要 Node.js：

```bash
npm install -g markdown-link-check
```

## 🚀 使用方式

### 本地运行

```bash
# 1. 元数据完整性检查
python scripts/doc-checks/check_metadata.py --root . --verbose

# 2. 内部链接检查
python scripts/doc-checks/check_internal_links.py --root . --verbose

# 3. 术语一致性检查
python scripts/doc-checks/check_terminology.py --root . --terminology docs/terminology.md

# 4. 版本号同步检查
python scripts/doc-checks/check_versions.py --root . --config scripts/doc-checks/version-config.yaml

# 5. 双语同步检查
python scripts/doc-checks/check_bilingual.py --root . --verbose
```

### GitHub Actions CI

所有检查会自动在以下情况触发：
- 推送到 `main` 或 `develop` 分支且修改了 `.md` 文件
- 针对 `main` 分支的 Pull Request
- 每周一定时运行
- 手动触发（支持选择检查类型）

## 📋 检查项说明

### 1. 元数据完整性检查 (`check_metadata.py`)

检查所有 Markdown 文件的 YAML Front Matter：

- 必需字段：title, description, last_updated
- 日期格式：YYYY-MM-DD
- 状态值有效性：design-phase, active, deprecated 等
- 语言标记：en, zh, bilingual

**输出格式**：
```bash
python scripts/doc-checks/check_metadata.py --format github  # GitHub Actions 格式
python scripts/doc-checks/check_metadata.py --output report.json  # JSON 报告
```

### 2. 内部链接检查 (`check_internal_links.py`)

检查文档中的内部链接：

- 相对路径链接是否存在
- 锚点链接是否有效
- 图片引用是否存在
- 交叉引用一致性

**特点**：
- 自动提取所有 Markdown 文件的标题锚点
- 支持 GitHub 风格锚点生成
- 排除代码块内的链接

### 3. 术语一致性检查 (`check_terminology.py`)

检查文档术语使用是否一致：

- 禁止术语（如 `engine_lua` → `engine-lua`）
- 中文翻译一致性
- 大小写敏感性检查
- Feature Flag 格式

**配置**：
默认使用 `docs/terminology.md` 中的术语规范，或内置默认规范。

### 4. 版本号同步检查 (`check_versions.py`)

检查文档中版本号是否一致：

- Rust 版本（1.83+）
- Node.js 版本（20+）
- Python 版本（3.10+）
- 项目版本（0.1.0）

**配置**：`version-config.yaml`

```yaml
versions:
  rust: "1.83"
  nodejs: "20"
  python: "3.10"
  project: "0.1.0"
```

### 5. 双语同步检查 (`check_bilingual.py`)

检查双语（中英文）文档的同步性：

- 章节结构是否对应
- 代码示例是否一致
- 代码块语言类型是否匹配

**检测方式**：
- 识别 `<a name="chinese">` 等标记
- 按中文章节标题自动分割

### 6. 文档时间戳更新 (`update_timestamps.py`)

自动更新文档的 `last_updated` 字段：

```bash
# 检查过期文档（7天未更新）
python scripts/doc-checks/update_timestamps.py --check-only --max-age-days 7

# 更新所有过期文档
python scripts/doc-checks/update_timestamps.py --files stale-files.txt
```

## 📊 CI 工作流

工作流文件：`.github/workflows/doc-quality.yml`

### 触发条件

```yaml
on:
  push:
    branches: [main, develop]
    paths: ['**.md']
  pull_request:
    branches: [main]
    paths: ['**.md']
  schedule:
    - cron: '0 0 * * 1'  # 每周一
  workflow_dispatch:
```

### Job 说明

| Job | 说明 | 触发条件 |
|-----|------|----------|
| `check-metadata` | 元数据完整性 | 所有触发 |
| `check-links` | 内部/外部链接 | 所有触发 |
| `check-terminology` | 术语一致性 | 所有触发 |
| `check-versions` | 版本号同步 | 所有触发 |
| `check-bilingual` | 双语同步 | 所有触发 |
| `generate-report` | 综合报告 | 所有检查完成后 |
| `update-timestamps` | 更新时间戳 | main 分支推送 |

## 📝 报告输出

### 控制台输出示例

```
============================================================
📋 元数据完整性检查报告
============================================================

统计:
  - 检查文件数: 41
  - 通过: 27
  - 失败: 13
  - 跳过: 0

❌ 错误 (13):
  docs/adr/001-architecture-layers.md:
    - 缺少必需字段: description

⚠️  警告 (1):
  docs/technical-feasibility-analysis.md:
    - 未知的状态值: completed
```

### GitHub Actions 输出

使用 `--format github` 会输出 GitHub Actions 可识别的格式：

```
::error file=docs/adr/001-architecture-layers.md,title=Metadata Error::缺少必需字段: description
::warning file=docs/technical-feasibility-analysis.md,title=Metadata Warning::未知的状态值: completed
```

这些输出会在 PR 中显示为注释。

## 🔧 故障排除

### 1. Python 版本

脚本需要 Python 3.10+：

```bash
python3 --version  # 应 >= 3.8
```

### 2. 权限问题

确保脚本可执行：

```bash
chmod +x scripts/doc-checks/*.py
```

### 3. YAML 解析警告

如果没有安装 `pyyaml`，脚本会使用简单的 YAML 解析器。安装 `pyyaml` 以获得更好的兼容性：

```bash
pip install pyyaml
```

## 🎯 最佳实践

1. **在提交前运行检查**
   ```bash
   python scripts/doc-checks/check_metadata.py --root .
   python scripts/doc-checks/check_internal_links.py --root .
   ```

2. **修复 P0 级别问题**
   - 元数据缺失
   - 死链接
   - 版本号冲突

3. **定期处理警告**
   - 术语使用建议
   - 双语同步提醒
   - 时间戳更新

4. **配置豁免**
   某些文件可以豁免检查（如 LICENSE 文件），在脚本中已内置豁免列表。

## 📚 参考

- [GitHub Actions 文档](https://docs.github.com/en/actions)
- [Markdown 链接检查工具](https://github.com/tcort/markdown-link-check)
- [YAML Front Matter 规范](https://jekyllrb.com/docs/front-matter/)
