#!/usr/bin/env python3
"""
元数据完整性检查脚本
检查所有 Markdown 文件的 YAML Front Matter 完整性

检查项：
1. YAML Front Matter 是否存在
2. 必需字段是否完整 (title, description, status, version, last_updated)
3. 字段值格式是否正确
4. 日期格式是否统一
"""

import argparse
import json
import os
import re
import sys
from datetime import datetime
from pathlib import Path
from typing import Dict, List, Optional, Tuple

try:
    import yaml
except ImportError:
    yaml = None


class MetadataChecker:
    """元数据检查器"""
    
    # 必需字段（按文档类型）
    REQUIRED_FIELDS = {
        "default": ["title", "description", "last_updated"],
        "root": ["title", "description", "status", "version", "last_updated"],
        "adr": ["title", "description", "status", "date"],
    }
    
    # 有效状态值
    VALID_STATUSES = [
        "design-phase", "active", "deprecated", "superseded",
        "proposed", "accepted", "rejected"
    ]
    
    # 有效语言值
    VALID_LANGUAGES = ["en", "zh", "bilingual"]
    
    def __init__(self, root_dir: str, verbose: bool = False):
        self.root_dir = Path(root_dir)
        self.verbose = verbose
        self.errors: List[Dict] = []
        self.warnings: List[Dict] = []
        self.stats = {"checked": 0, "passed": 0, "failed": 0, "skipped": 0}
        
    def log(self, message: str):
        """日志输出"""
        if self.verbose:
            print(message)
    
    def _simple_yaml_parse(self, content: str) -> Dict:
        """简单的 YAML 解析（仅支持基本键值对）"""
        result = {}
        for line in content.strip().split('\n'):
            line = line.strip()
            if not line or line.startswith('#'):
                continue
            if ':' in line:
                key, value = line.split(':', 1)
                key = key.strip()
                value = value.strip().strip('"\'')
                result[key] = value
        return result
    
    def get_doc_type(self, file_path: Path) -> str:
        """根据文件路径判断文档类型"""
        rel_path = file_path.relative_to(self.root_dir)
        
        if "adr" in str(rel_path).lower():
            return "adr"
        elif rel_path.parent == Path("."):
            return "root"
        else:
            return "default"
    
    def parse_front_matter(self, content: str) -> Tuple[Optional[Dict], str]:
        """解析 YAML Front Matter"""
        if not content.startswith("---"):
            return None, content
        
        # 查找结束标记
        end_match = re.search(r"\n---\s*\n", content[3:])
        if not end_match:
            return None, content
        
        yaml_content = content[3:3 + end_match.start()]
        body_content = content[3 + end_match.end():]
        
        try:
            if yaml:
                metadata = yaml.safe_load(yaml_content)
            else:
                # 简单的 YAML 解析回退
                metadata = self._simple_yaml_parse(yaml_content)
            return metadata, body_content
        except Exception as e:
            return {"_error": str(e)}, body_content
    
    def check_date_format(self, date_str: str) -> bool:
        """检查日期格式是否为 YYYY-MM-DD"""
        try:
            datetime.strptime(date_str, "%Y-%m-%d")
            return True
        except (ValueError, TypeError):
            return False
    
    def check_file(self, file_path: Path) -> Dict:
        """检查单个文件"""
        result = {
            "file": str(file_path.relative_to(self.root_dir)),
            "passed": True,
            "errors": [],
            "warnings": [],
            "metadata": None
        }
        
        try:
            content = file_path.read_text(encoding="utf-8")
        except Exception as e:
            result["passed"] = False
            result["errors"].append(f"无法读取文件: {e}")
            return result
        
        metadata, body = self.parse_front_matter(content)
        
        # 检查是否有 YAML Front Matter
        if metadata is None:
            # 某些文件可以豁免（如 CHANGELOG, LICENSE）
            # 豁免列表：展示文档或辅助文件
            exempt_files = {"CHANGELOG.md", "LICENSE-MIT", "LICENSE-APACHE",
                           "README.md", "SECURITY.md", "CODE_OF_CONDUCT.md", "CONTRIBUTING.md"}
            exempt_dirs = {Path("examples"), Path("scripts")}
            
            rel_path = file_path.relative_to(self.root_dir)
            if file_path.name in exempt_files or any(part in exempt_dirs for part in rel_path.parents) or rel_path.parts[0] in ["examples", "scripts"]:
                self.stats["skipped"] += 1
                result["passed"] = True
                result["warnings"].append("文件被豁免元数据检查")
                return result
            
            result["passed"] = False
            result["errors"].append("缺少 YAML Front Matter")
            return result
        
        # 检查 YAML 解析错误
        if "_error" in metadata:
            result["passed"] = False
            result["errors"].append(f"YAML 解析错误: {metadata['_error']}")
            return result
        
        result["metadata"] = metadata
        doc_type = self.get_doc_type(file_path)
        required_fields = self.REQUIRED_FIELDS.get(doc_type, self.REQUIRED_FIELDS["default"])
        
        # 检查必需字段
        for field in required_fields:
            if field not in metadata or metadata[field] is None:
                result["passed"] = False
                result["errors"].append(f"缺少必需字段: {field}")
        
        # 检查字段值格式
        if "status" in metadata:
            if metadata["status"] not in self.VALID_STATUSES:
                result["warnings"].append(
                    f"未知的状态值: {metadata['status']}. "
                    f"建议使用: {', '.join(self.VALID_STATUSES[:4])}"
                )
        
        if "language" in metadata:
            if metadata["language"] not in self.VALID_LANGUAGES:
                result["warnings"].append(
                    f"未知的语言值: {metadata['language']}. "
                    f"建议使用: {', '.join(self.VALID_LANGUAGES)}"
                )
        
        # 检查日期格式
        date_fields = ["last_updated", "date"]
        for field in date_fields:
            if field in metadata and metadata[field]:
                if not self.check_date_format(str(metadata[field])):
                    result["passed"] = False
                    result["errors"].append(
                        f"字段 {field} 日期格式错误: {metadata[field]}, "
                        f"应为 YYYY-MM-DD"
                    )
        
        # 检查版本号格式
        if "version" in metadata and metadata["version"]:
            version = str(metadata["version"])
            if not re.match(r"^\d+\.\d+(\.\d+)?$", version):
                result["warnings"].append(f"版本号格式建议: {version} (建议使用语义化版本 x.y.z)")
        
        return result
    
    def check_all(self) -> Dict:
        """检查所有 Markdown 文件"""
        md_files = list(self.root_dir.rglob("*.md"))
        
        # 排除目录
        exclude_dirs = [".git", "node_modules", "target", ".venv", "venv", "__pycache__"]
        md_files = [
            f for f in md_files 
            if not any(excluded in str(f) for excluded in exclude_dirs)
        ]
        
        results = []
        for file_path in sorted(md_files):
            self.stats["checked"] += 1
            self.log(f"Checking: {file_path.relative_to(self.root_dir)}")
            
            result = self.check_file(file_path)
            results.append(result)
            
            if result["passed"] and not result["warnings"]:
                self.stats["passed"] += 1
            elif not result["passed"]:
                self.stats["failed"] += 1
                self.errors.append({
                    "file": result["file"],
                    "errors": result["errors"]
                })
            
            if result["warnings"]:
                self.warnings.append({
                    "file": result["file"],
                    "warnings": result["warnings"]
                })
        
        return {
            "stats": self.stats,
            "errors": self.errors,
            "warnings": self.warnings,
            "details": results
        }
    
    def print_report(self, format_type: str = "console"):
        """输出报告"""
        if format_type == "github":
            self._print_github_format()
        else:
            self._print_console_format()
    
    def _print_console_format(self):
        """控制台格式输出"""
        print("=" * 60)
        print("📋 元数据完整性检查报告")
        print("=" * 60)
        print(f"\n统计:")
        print(f"  - 检查文件数: {self.stats['checked']}")
        print(f"  - 通过: {self.stats['passed']}")
        print(f"  - 失败: {self.stats['failed']}")
        print(f"  - 跳过: {self.stats['skipped']}")
        
        if self.errors:
            print(f"\n❌ 错误 ({len(self.errors)}):")
            for error in self.errors:
                print(f"\n  {error['file']}:")
                for msg in error["errors"]:
                    print(f"    - {msg}")
        
        if self.warnings:
            print(f"\n⚠️  警告 ({len(self.warnings)}):")
            for warning in self.warnings:
                print(f"\n  {warning['file']}:")
                for msg in warning["warnings"]:
                    print(f"    - {msg}")
        
        print("\n" + "=" * 60)
    
    def _print_github_format(self):
        """GitHub Actions 格式输出"""
        for error in self.errors:
            for msg in error["errors"]:
                print(f"::error file={error['file']},title=Metadata Error::{msg}")
        
        for warning in self.warnings:
            for msg in warning["warnings"]:
                print(f"::warning file={warning['file']},title=Metadata Warning::{msg}")


def main():
    parser = argparse.ArgumentParser(description="检查 Markdown 文件元数据完整性")
    parser.add_argument("--root", "-r", default=".", help="项目根目录")
    parser.add_argument("--format", "-f", choices=["console", "github"], 
                       default="console", help="输出格式")
    parser.add_argument("--output", "-o", help="JSON 报告输出路径")
    parser.add_argument("--verbose", "-v", action="store_true", help="详细输出")
    
    args = parser.parse_args()
    
    checker = MetadataChecker(args.root, args.verbose)
    results = checker.check_all()
    
    checker.print_report(args.format)
    
    # 输出 JSON 报告
    if args.output:
        with open(args.output, "w", encoding="utf-8") as f:
            json.dump(results, f, ensure_ascii=False, indent=2)
        print(f"\n📄 报告已保存: {args.output}")
    
    # 返回退出码
    sys.exit(0 if not checker.errors else 1)


if __name__ == "__main__":
    main()
