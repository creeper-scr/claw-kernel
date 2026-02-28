#!/usr/bin/env python3
"""
综合报告生成脚本
整合所有检查结果生成统一的报告
"""

import argparse
import json
import sys
from datetime import datetime
from pathlib import Path
from typing import Dict, List


class ReportGenerator:
    """报告生成器"""
    
    def __init__(self, input_dir: str, output_file: str):
        self.input_dir = Path(input_dir)
        self.output_file = Path(output_file)
        self.reports: Dict[str, Dict] = {}
        
    def load_reports(self):
        """加载所有检查报告"""
        report_files = {
            "metadata": "metadata-report/metadata-report.json",
            "links": "link-report/link-report.json",
            "terminology": "terminology-report/terminology-report.json",
            "versions": "version-report/version-report.json",
            "bilingual": "bilingual-report/bilingual-report.json",
        }
        
        for name, filename in report_files.items():
            filepath = self.input_dir / filename
            if filepath.exists():
                try:
                    with open(filepath, "r", encoding="utf-8") as f:
                        self.reports[name] = json.load(f)
                except Exception as e:
                    print(f"警告: 无法加载 {name} 报告: {e}")
    
    def generate_markdown_report(self) -> str:
        """生成 Markdown 格式报告"""
        now = datetime.now().strftime("%Y-%m-%d %H:%M:%S")
        
        lines = [
            "# 📊 文档质量检查报告",
            "",
            f"**生成时间**: {now}",
            f"**项目**: claw-kernel",
            "",
            "---",
            "",
        ]
        
        # 汇总统计
        total_errors = 0
        total_warnings = 0
        total_checked = 0
        
        for report in self.reports.values():
            total_errors += len(report.get("errors", []))
            total_warnings += len(report.get("warnings", []))
            total_checked += report.get("stats", {}).get("checked", 0)
        
        lines.extend([
            "## 📈 汇总统计",
            "",
            f"| 指标 | 数量 |",
            f"|------|------|",
            f"| 检查文件数 | {total_checked} |",
            f"| 错误总数 | {total_errors} |",
            f"| 警告总数 | {total_warnings} |",
            "",
        ])
        
        # 各检查项详情
        lines.extend([
            "## 📋 详细检查结果",
            "",
        ])
        
        check_names = {
            "metadata": "元数据完整性",
            "links": "链接有效性",
            "terminology": "术语一致性",
            "versions": "版本号同步",
            "bilingual": "双语同步",
        }
        
        for key, name in check_names.items():
            if key not in self.reports:
                continue
            
            report = self.reports[key]
            errors = report.get("errors", [])
            warnings = report.get("warnings", [])
            stats = report.get("stats", {})
            
            status_icon = "✅" if not errors else "❌"
            lines.extend([
                f"### {status_icon} {name}",
                "",
            ])
            
            # 统计
            lines.append(f"**统计**: 检查 {stats.get('checked', 0)} 个文件")
            if errors:
                lines.append(f", 发现 {len(errors)} 个错误")
            if warnings:
                lines.append(f", {len(warnings)} 个警告")
            lines.append("\n")
            
            # 错误详情
            if errors:
                lines.extend([
                    "**错误**:",
                    "",
                ])
                for error in errors[:5]:  # 只显示前5个
                    file_path = error.get("file", "N/A")
                    line = error.get("line", 0)
                    message = error.get("message", "")
                    lines.append(f"- `{file_path}:{line}`: {message}")
                if len(errors) > 5:
                    lines.append(f"- ... 还有 {len(errors) - 5} 个错误")
                lines.append("")
            
            # 警告详情
            if warnings:
                lines.extend([
                    "**警告**:",
                    "",
                ])
                for warning in warnings[:3]:  # 只显示前3个
                    file_path = warning.get("file", "N/A")
                    line = warning.get("line", 0)
                    message = warning.get("message", "")
                    lines.append(f"- `{file_path}:{line}`: {message}")
                if len(warnings) > 3:
                    lines.append(f"- ... 还有 {len(warnings) - 3} 个警告")
                lines.append("")
        
        # 修复建议
        lines.extend([
            "## 🔧 修复建议",
            "",
        ])
        
        if total_errors == 0:
            lines.extend([
                "✅ 所有检查通过！没有需要修复的问题。",
                "",
            ])
        else:
            lines.extend([
                "根据检查结果，建议按以下优先级修复：",
                "",
                "### 高优先级（错误）",
                "1. 修复所有死链接",
                "2. 统一版本号声明",
                "3. 补充缺失的 YAML Front Matter",
                "",
                "### 中优先级（警告）",
                "1. 统一术语使用",
                "2. 同步双语文档内容",
                "3. 更新过时的文档时间戳",
                "",
            ])
        
        # 添加 CI 信息
        lines.extend([
            "---",
            "",
            "*此报告由文档质量 CI 自动生成*",
        ])
        
        return "\n".join(lines)
    
    def generate_html_report(self) -> str:
        """生成 HTML 格式报告"""
        # 简化版本，实际使用可以添加更多样式
        md_content = self.generate_markdown_report()
        # 这里可以转换为 HTML
        return md_content
    
    def save_report(self, content: str):
        """保存报告"""
        self.output_file.write_text(content, encoding="utf-8")
        print(f"报告已保存: {self.output_file}")


def main():
    parser = argparse.ArgumentParser(description="生成文档质量综合报告")
    parser.add_argument("--input-dir", "-i", required=True, help="输入报告目录")
    parser.add_argument("--output", "-o", required=True, help="输出报告路径")
    parser.add_argument("--format", "-f", choices=["markdown", "html"], 
                       default="markdown", help="输出格式")
    
    args = parser.parse_args()
    
    generator = ReportGenerator(args.input_dir, args.output)
    generator.load_reports()
    
    if args.format == "html":
        content = generator.generate_html_report()
    else:
        content = generator.generate_markdown_report()
    
    generator.save_report(content)


if __name__ == "__main__":
    main()
