#!/usr/bin/env python3
"""
版本号同步检查脚本
检查文档中版本号是否一致

检查项：
1. Rust 版本号一致性
2. Node.js 版本号一致性
3. Python 版本号一致性
4. 项目版本号一致性
"""

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Dict, List, Optional, Tuple


class VersionChecker:
    """版本号检查器"""
    
    # 预期的版本号（从配置文件读取）
    EXPECTED_VERSIONS = {
        "rust": "1.83",
        "nodejs": "20",
        "python": "3.10",
        "project": "0.1.0",
    }
    
    # 版本号出现的位置模式
    VERSION_PATTERNS = {
        "rust": [
            (r'Rust[^\d]*(\d+\.\d+)', "Rust 版本声明"),
            (r'rustc[^\d]*(\d+\.\d+)', "rustc 版本"),
            (r'channel\s*=\s*"(\d+\.\d+)"', "rust-toolchain.toml"),
            (r'MSRV[^\d]*(\d+\.\d+)', "MSRV 声明"),
        ],
        "nodejs": [
            (r'Node\.js[^\d]*(\d+)', "Node.js 版本声明"),
            (r'node[^\d]*(\d+)', "Node 版本"),
        ],
        "python": [
            (r'Python[^\d]*(\d+\.\d+)', "Python 版本声明"),
            (r'pyo3.*?(\d+\.\d+)', "PyO3 相关版本"),
        ],
        "project": [
            (r'version:\s*"(\d+\.\d+\.?\d*)"', "YAML 版本字段"),
            (r'claw-kernel[^\d]*(\d+\.\d+\.?\d*)', "crate 版本引用"),
        ],
    }
    
    def __init__(self, root_dir: str, config_file: Optional[str] = None,
                 verbose: bool = False):
        self.root_dir = Path(root_dir)
        self.verbose = verbose
        self.errors: List[Dict] = []
        self.warnings: List[Dict] = []
        self.stats = {"checked": 0, "mismatches": 0}
        
        # 加载配置
        if config_file:
            self._load_config(config_file)
    
    def log(self, message: str):
        """日志输出"""
        if self.verbose:
            print(message)
    
    def _load_config(self, config_file: str):
        """从配置文件加载版本规范"""
        try:
            import yaml
            config = yaml.safe_load(Path(config_file).read_text())
            if config and "versions" in config:
                self.EXPECTED_VERSIONS.update(config["versions"])
        except Exception as e:
            print(f"警告: 无法加载配置文件: {e}")
    
    def extract_versions(self, content: str, file_path: Path) -> List[Tuple[str, str, str, int]]:
        """
        从内容中提取版本号
        返回: [(版本类型, 版本号, 描述, 行号), ...]
        """
        versions = []
        lines = content.split('\n')
        
        for version_type, patterns in self.VERSION_PATTERNS.items():
            for pattern, desc in patterns:
                for line_num, line in enumerate(lines, 1):
                    matches = re.finditer(pattern, line, re.IGNORECASE)
                    for match in matches:
                        version = match.group(1)
                        versions.append((version_type, version, desc, line_num))
        
        return versions
    
    def check_file(self, file_path: Path) -> List[Dict]:
        """检查单个文件"""
        issues = []
        
        try:
            content = file_path.read_text(encoding="utf-8")
        except Exception as e:
            issues.append({
                "file": str(file_path.relative_to(self.root_dir)),
                "line": 0,
                "type": "error",
                "message": f"无法读取文件: {e}"
            })
            return issues
        
        rel_path = str(file_path.relative_to(self.root_dir))
        versions = self.extract_versions(content, file_path)
        
        for version_type, found_version, desc, line_num in versions:
            expected = self.EXPECTED_VERSIONS.get(version_type)
            if not expected:
                continue
            
            # 检查是否匹配（允许 + 后缀，如 "1.83+", "20 LTS"）
            clean_found = re.match(r'(\d+\.?\d*)', found_version)
            clean_expected = re.match(r'(\d+\.?\d*)', expected)
            
            if clean_found and clean_expected:
                found_base = clean_found.group(1)
                expected_base = clean_expected.group(1)
                
                # 版本号不匹配
                if not found_base.startswith(expected_base) and \
                   not expected_base.startswith(found_base):
                    issues.append({
                        "file": rel_path,
                        "line": line_num,
                        "type": "error",
                        "version_type": version_type,
                        "found": found_version,
                        "expected": expected,
                        "message": f"{desc} 版本号不一致: 找到 '{found_version}', 期望 '{expected}'"
                    })
                    self.stats["mismatches"] += 1
                # 版本号匹配但格式可能有问题（警告级别）
                elif found_version != expected and \
                     not found_version.startswith(expected + "+") and \
                     not found_version.startswith(expected + " "):
                    issues.append({
                        "file": rel_path,
                        "line": line_num,
                        "type": "warning",
                        "version_type": version_type,
                        "found": found_version,
                        "expected": expected,
                        "message": f"{desc} 版本号格式建议: '{found_version}' -> '{expected}+' 或 '{expected} LTS'"
                    })
        
        self.stats["checked"] += 1
        return issues
    
    def check_special_files(self) -> List[Dict]:
        """检查特殊文件（如 Cargo.toml, rust-toolchain.toml）"""
        issues = []
        
        # 检查 rust-toolchain.toml
        toolchain_file = self.root_dir / "rust-toolchain.toml"
        if toolchain_file.exists():
            content = toolchain_file.read_text(encoding="utf-8")
            match = re.search(r'channel\s*=\s*"(\d+\.\d+)"', content)
            if match:
                found = match.group(1)
                expected = self.EXPECTED_VERSIONS["rust"]
                if not found.startswith(expected):
                    issues.append({
                        "file": "rust-toolchain.toml",
                        "line": 0,
                        "type": "error",
                        "version_type": "rust",
                        "found": found,
                        "expected": expected,
                        "message": f"rust-toolchain.toml 中的 Rust 版本 '{found}' 与期望版本 '{expected}' 不一致"
                    })
                    self.stats["mismatches"] += 1
        
        # 检查 Cargo.toml
        cargo_file = self.root_dir / "Cargo.toml"
        if cargo_file.exists():
            content = cargo_file.read_text(encoding="utf-8")
            
            # 提取 rust-version
            match = re.search(r'rust-version\s*=\s*"(\d+\.\d+)"', content)
            if match:
                found = match.group(1)
                expected = self.EXPECTED_VERSIONS["rust"]
                if not found.startswith(expected):
                    issues.append({
                        "file": "Cargo.toml",
                        "line": 0,
                        "type": "error",
                        "version_type": "rust",
                        "found": found,
                        "expected": expected,
                        "message": f"Cargo.toml 中的 rust-version '{found}' 与期望版本 '{expected}' 不一致"
                    })
                    self.stats["mismatches"] += 1
            else:
                issues.append({
                    "file": "Cargo.toml",
                    "line": 0,
                    "type": "warning",
                    "message": "Cargo.toml 中缺少 rust-version 字段"
                })
        
        return issues
    
    def check_all(self) -> Dict:
        """检查所有 Markdown 文件"""
        exclude_dirs = {".git", "node_modules", "target", ".venv", "venv", "__pycache__"}
        
        md_files = [
            f for f in self.root_dir.rglob("*.md")
            if not any(excluded in str(f) for excluded in exclude_dirs)
        ]
        
        all_issues = []
        
        # 检查特殊文件
        special_issues = self.check_special_files()
        all_issues.extend(special_issues)
        
        # 检查 Markdown 文件
        for file_path in sorted(md_files):
            self.log(f"Checking: {file_path.relative_to(self.root_dir)}")
            issues = self.check_file(file_path)
            all_issues.extend(issues)
        
        # 分类问题
        for issue in all_issues:
            if issue.get("type") == "error":
                self.errors.append(issue)
            else:
                self.warnings.append(issue)
        
        return {
            "expected_versions": self.EXPECTED_VERSIONS,
            "stats": self.stats,
            "errors": self.errors,
            "warnings": self.warnings
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
        print("🔢 版本号一致性检查报告")
        print("=" * 60)
        print(f"\n期望版本:")
        for k, v in self.EXPECTED_VERSIONS.items():
            print(f"  - {k}: {v}")
        print(f"\n统计:")
        print(f"  - 检查文件数: {self.stats['checked']}")
        print(f"  - 版本不匹配: {self.stats['mismatches']}")
        
        if self.errors:
            print(f"\n❌ 错误 ({len(self.errors)}):")
            for error in self.errors[:15]:
                print(f"\n  {error['file']}:{error.get('line', 0)}")
                print(f"    {error['message']}")
        
        if self.warnings:
            print(f"\n⚠️  警告 ({len(self.warnings)}):")
            for warning in self.warnings[:10]:
                print(f"\n  {warning['file']}:{warning.get('line', 0)}")
                print(f"    {warning['message']}")
        
        print("\n" + "=" * 60)
    
    def _print_github_format(self):
        """GitHub Actions 格式输出"""
        for error in self.errors:
            line = error.get('line', 0)
            print(f"::error file={error['file']},line={line},title=Version Mismatch::{error['message']}")
        
        for warning in self.warnings:
            line = warning.get('line', 0)
            print(f"::warning file={warning['file']},line={line},title=Version Warning::{warning['message']}")


def main():
    parser = argparse.ArgumentParser(description="检查文档版本号一致性")
    parser.add_argument("--root", "-r", default=".", help="项目根目录")
    parser.add_argument("--config", "-c", help="版本配置 YAML 文件")
    parser.add_argument("--format", "-f", choices=["console", "github"], 
                       default="console", help="输出格式")
    parser.add_argument("--output", "-o", help="JSON 报告输出路径")
    parser.add_argument("--verbose", "-v", action="store_true", help="详细输出")
    
    args = parser.parse_args()
    
    checker = VersionChecker(args.root, args.config, args.verbose)
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
