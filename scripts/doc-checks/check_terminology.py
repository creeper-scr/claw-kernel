#!/usr/bin/env python3
"""
术语一致性检查脚本
检查文档中术语的使用是否一致

检查项：
1. 禁止术语（如 engine_lua, 热重载）
2. 中文翻译一致性
3. 大小写一致性
4. Feature Flag 格式
"""

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Dict, List, Set, Tuple


class TerminologyChecker:
    """术语检查器"""
    
    # 默认术语规范（当没有术语表文件时使用）
    DEFAULT_TERMS = {
        # 禁止使用的术语
        "forbidden": {
            "engine_lua": "应使用 engine-lua（连字符格式）",
            "热重载": "应使用 热加载 (Hot-loading)",
            "六层架构": "应使用 五层架构 (Five-Layer Architecture)",
            "沙盒": "应使用 沙箱 (Sandbox)",
        },
        # 必须一致的术语映射
        "consistent": {
            # 英文: 标准中文翻译
            "Power Mode": "强力模式 (Power Mode)",
            "Safe Mode": "安全模式 (Safe Mode)",
            "Hot-loading": "热加载 (Hot-loading)",
            "Sandbox": "沙箱 (Sandbox)",
        },
        # 大小写敏感的关键术语
        "case_sensitive": [
            "claw-kernel",
            "claw-pal",
            "claw-provider",
            "claw-tools",
            "claw-loop",
            "claw-runtime",
            "claw-script",
        ]
    }
    
    def __init__(self, root_dir: str, terminology_file: Optional[str] = None, 
                 verbose: bool = False):
        self.root_dir = Path(root_dir)
        self.verbose = verbose
        self.errors: List[Dict] = []
        self.warnings: List[Dict] = []
        self.stats = {"checked": 0, "issues": 0}
        
        # 加载术语规范
        self.terms = self._load_terminology(terminology_file)
    
    def log(self, message: str):
        """日志输出"""
        if self.verbose:
            print(message)
    
    def _load_terminology(self, terminology_file: Optional[str]) -> Dict:
        """从文件加载术语规范"""
        if terminology_file and Path(terminology_file).exists():
            try:
                content = Path(terminology_file).read_text(encoding="utf-8")
                return self._parse_terminology_md(content)
            except Exception as e:
                print(f"警告: 无法加载术语表: {e}，使用默认规范")
        
        return self.DEFAULT_TERMS
    
    def _parse_terminology_md(self, content: str) -> Dict:
        """从 Markdown 术语表解析术语规范"""
        terms = {
            "forbidden": {},
            "consistent": {},
            "case_sensitive": []
        }
        
        # 解析禁止用法表格
        # | No 避免 | Yes 使用 |
        forbidden_pattern = r'\|\s*No\s+避免\s*\|\s*Yes\s+使用\s*\|[^|]*\|\n((?:\|[^\|]+\|[^\|]+\|[^\|]*\|\n)+)'
        forbidden_match = re.search(forbidden_pattern, content)
        if forbidden_match:
            for line in forbidden_match.group(1).strip().split('\n'):
                parts = [p.strip() for p in line.split('|')]
                if len(parts) >= 3:
                    bad_term = parts[1]
                    good_term = parts[2]
                    if bad_term and good_term and bad_term != "-":
                        terms["forbidden"][bad_term] = f"应使用 {good_term}"
        
        # 解析术语对照表
        # | 英文 | 中文 | 备注 |
        term_pattern = r'\|\s*英文[^|]*\|\s*中文[^|]*\|[^|]*\|\n((?:\|[^\|]+\|[^\|]+\|[^\|]*\|\n)+)'
        for match in re.finditer(term_pattern, content):
            for line in match.group(1).strip().split('\n'):
                parts = [p.strip() for p in line.split('|')]
                if len(parts) >= 3:
                    en_term = parts[1]
                    zh_term = parts[2]
                    if en_term and zh_term and not en_term.startswith('-'):
                        terms["consistent"][en_term] = zh_term
        
        return terms
    
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
        lines = content.split('\n')
        
        # 检查禁止术语
        for bad_term, suggestion in self.terms.get("forbidden", {}).items():
            pattern = re.compile(re.escape(bad_term), re.IGNORECASE)
            
            for line_num, line in enumerate(lines, 1):
                if pattern.search(line):
                    # 排除代码块和 YAML front matter
                    if self._is_in_code_block(lines, line_num - 1):
                        continue
                    
                    issues.append({
                        "file": rel_path,
                        "line": line_num,
                        "type": "error",
                        "message": f"发现禁止术语: '{bad_term}' -> {suggestion}",
                        "context": line.strip()[:80]
                    })
                    self.stats["issues"] += 1
        
        # 检查大小写敏感术语
        for term in self.terms.get("case_sensitive", []):
            wrong_cases = [
                term.upper(),  # CLAW-KERNEL
                term.lower(),  # claw-kernel（如果原始包含大写）
                term.replace('-', '_'),  # claw_kernel
                term.title(),  # Claw-Kernel
            ]
            
            for wrong in set(wrong_cases):
                if wrong == term:
                    continue
                
                pattern = re.compile(r'\b' + re.escape(wrong) + r'\b')
                
                for line_num, line in enumerate(lines, 1):
                    if pattern.search(line):
                        if self._is_in_code_block(lines, line_num - 1):
                            continue
                        
                        issues.append({
                            "file": rel_path,
                            "line": line_num,
                            "type": "warning",
                            "message": f"大小写不一致: '{wrong}' 应使用 '{term}'",
                            "context": line.strip()[:80]
                        })
                        self.stats["issues"] += 1
        
        # 检查术语使用一致性
        # 找出中英文混用不一致的情况
        zh_pattern = re.compile(r'[\u4e00-\u9fff]+')
        has_chinese = bool(zh_pattern.search(content))
        
        if has_chinese:
            # 检查是否在中文章节使用了英文术语但未标注
            for en_term, zh_term in self.terms.get("consistent", {}).items():
                # 简单启发式：如果行中有中文，且使用了英文术语
                pattern = re.compile(r'\b' + re.escape(en_term) + r'\b')
                
                for line_num, line in enumerate(lines, 1):
                    if zh_pattern.search(line) and pattern.search(line):
                        # 检查是否已经有中文标注
                        # 如果行中已经有对应的中文，则不报错
                        if isinstance(zh_term, str) and zh_term.split()[0] in line:
                            continue
                        
                        # 跳过代码块和链接
                        if self._is_in_code_block(lines, line_num - 1):
                            continue
                        if re.search(r'\[.*?\]\(' + re.escape(en_term), line):
                            continue
                        
                        issues.append({
                            "file": rel_path,
                            "line": line_num,
                            "type": "warning",
                            "message": f"在中文章节中使用英文术语 '{en_term}'，建议标注中文: '{zh_term}'",
                            "context": line.strip()[:80]
                        })
        
        self.stats["checked"] += 1
        return issues
    
    def _is_in_code_block(self, lines: List[str], line_index: int) -> bool:
        """检查指定行是否在代码块内"""
        in_code_block = False
        for i, line in enumerate(lines):
            if i > line_index:
                break
            if line.strip().startswith('```'):
                in_code_block = not in_code_block
        return in_code_block
    
    def check_all(self) -> Dict:
        """检查所有 Markdown 文件"""
        exclude_dirs = {".git", "node_modules", "target", ".venv", "venv", "__pycache__"}
        
        md_files = [
            f for f in self.root_dir.rglob("*.md")
            if not any(excluded in str(f) for excluded in exclude_dirs)
        ]
        
        all_issues = []
        for file_path in sorted(md_files):
            self.log(f"Checking: {file_path.relative_to(self.root_dir)}")
            issues = self.check_file(file_path)
            all_issues.extend(issues)
        
        # 分类问题
        for issue in all_issues:
            if issue["type"] == "error":
                self.errors.append(issue)
            else:
                self.warnings.append(issue)
        
        return {
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
        print("📚 术语一致性检查报告")
        print("=" * 60)
        print(f"\n统计:")
        print(f"  - 检查文件数: {self.stats['checked']}")
        print(f"  - 发现问题数: {self.stats['issues']}")
        
        if self.errors:
            print(f"\n❌ 错误 ({len(self.errors)}):")
            for error in self.errors[:15]:
                print(f"\n  {error['file']}:{error['line']}")
                print(f"    {error['message']}")
                print(f"    上下文: {error.get('context', 'N/A')}")
        
        if self.warnings:
            print(f"\n⚠️  警告 ({len(self.warnings)}):")
            for warning in self.warnings[:10]:
                print(f"\n  {warning['file']}:{warning['line']}")
                print(f"    {warning['message']}")
        
        print("\n" + "=" * 60)
    
    def _print_github_format(self):
        """GitHub Actions 格式输出"""
        for error in self.errors:
            print(f"::error file={error['file']},line={error['line']},title=Terminology Error::{error['message']}")
        
        for warning in self.warnings:
            print(f"::warning file={warning['file']},line={warning['line']},title=Terminology Warning::{warning['message']}")


def main():
    parser = argparse.ArgumentParser(description="检查文档术语一致性")
    parser.add_argument("--root", "-r", default=".", help="项目根目录")
    parser.add_argument("--terminology", "-t", help="术语表文件路径")
    parser.add_argument("--format", "-f", choices=["console", "github"], 
                       default="console", help="输出格式")
    parser.add_argument("--output", "-o", help="JSON 报告输出路径")
    parser.add_argument("--verbose", "-v", action="store_true", help="详细输出")
    
    args = parser.parse_args()
    
    checker = TerminologyChecker(args.root, args.terminology, args.verbose)
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
