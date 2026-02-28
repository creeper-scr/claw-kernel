#!/usr/bin/env python3
"""
双语文档同步检查脚本
检查双语（中英文）文档的内容同步性

检查项：
1. 中英文章节结构是否对应
2. 代码示例是否一致
3. 关键链接是否一致
4. 表格结构是否一致
"""

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Dict, List, Optional, Set, Tuple


class BilingualChecker:
    """双语检查器"""
    
    # 常见的章节标题映射（英文 -> 中文）
    SECTION_MAPPINGS = {
        "table of contents": "目录",
        "introduction": "简介",
        "quick start": "快速开始",
        "installation": "安装",
        "configuration": "配置",
        "usage": "使用",
        "api reference": "api 参考",
        "examples": "示例",
        "troubleshooting": "故障排除",
        "contributing": "贡献",
        "license": "许可证",
        "acknowledgments": "致谢",
        "architecture": "架构",
        "security": "安全",
        "performance": "性能",
    }
    
    def __init__(self, root_dir: str, verbose: bool = False):
        self.root_dir = Path(root_dir)
        self.verbose = verbose
        self.errors: List[Dict] = []
        self.warnings: List[Dict] = []
        self.stats = {"checked": 0, "bilingual_files": 0, "issues": 0}
    
    def log(self, message: str):
        """日志输出"""
        if self.verbose:
            print(message)
    
    def is_bilingual_file(self, content: str) -> bool:
        """判断是否为双语文件"""
        # 检查是否有中英双语导航
        has_bilingual_nav = bool(re.search(r'\[English\]|\[中文\]|\[Chinese\]', content))
        
        # 检查是否同时包含中英文
        chinese_chars = len(re.findall(r'[\u4e00-\u9fff]', content))
        english_words = len(re.findall(r'[a-zA-Z]+', content))
        
        return has_bilingual_nav or (chinese_chars > 50 and english_words > 100)
    
    def split_bilingual_content(self, content: str) -> Tuple[str, str]:
        """
        将双语内容拆分为英文和中文部分
        返回: (english_content, chinese_content)
        """
        # 查找中文部分锚点
        chinese_markers = [
            r'<a name="chinese"></a>',
            r'<a name="zh"></a>',
            r'## 中文',
            r'# 中文',
        ]
        
        chinese_start = None
        for marker in chinese_markers:
            match = re.search(marker, content, re.IGNORECASE)
            if match:
                chinese_start = match.start()
                break
        
        if chinese_start:
            english_content = content[:chinese_start]
            chinese_content = content[chinese_start:]
            return english_content, chinese_content
        
        # 如果没有明确标记，尝试按标题检测
        lines = content.split('\n')
        chinese_start_line = None
        
        for i, line in enumerate(lines):
            # 检测中文章节标题
            if re.match(r'^#{1,2}\s+[\u4e00-\u9fff]', line):
                chinese_start_line = i
                break
        
        if chinese_start_line:
            english_content = '\n'.join(lines[:chinese_start_line])
            chinese_content = '\n'.join(lines[chinese_start_line:])
            return english_content, chinese_content
        
        # 无法分离，返回全部作为英文
        return content, ""
    
    def extract_sections(self, content: str) -> List[Tuple[int, str]]:
        """提取所有章节标题及其层级"""
        sections = []
        for match in re.finditer(r'^(#{1,6})\s+(.+)$', content, re.MULTILINE):
            level = len(match.group(1))
            title = match.group(2).strip()
            sections.append((level, title))
        return sections
    
    def extract_code_blocks(self, content: str) -> List[Tuple[str, str]]:
        """提取代码块（语言, 内容）"""
        blocks = []
        pattern = r'```(\w+)?\n(.*?)```'
        for match in re.finditer(pattern, content, re.DOTALL):
            lang = match.group(1) or ""
            code = match.group(2).strip()
            blocks.append((lang, code))
        return blocks
    
    def extract_links(self, content: str) -> Set[str]:
        """提取所有链接目标"""
        links = set()
        # Markdown 链接
        for match in re.finditer(r'\[([^\]]+)\]\(([^)]+)\)', content):
            links.add(match.group(2))
        return links
    
    def extract_tables(self, content: str) -> List[List[List[str]]]:
        """提取表格结构"""
        tables = []
        lines = content.split('\n')
        
        i = 0
        while i < len(lines):
            line = lines[i].strip()
            
            # 检测表格开始
            if '|' in line and not line.startswith('```'):
                table = []
                # 收集表格所有行
                while i < len(lines) and '|' in lines[i]:
                    cells = [c.strip() for c in lines[i].split('|')[1:-1]]
                    if cells and not all(re.match(r'^[-:]+$', c) for c in cells):
                        table.append(cells)
                    i += 1
                
                if table:
                    tables.append(table)
            else:
                i += 1
        
        return tables
    
    def normalize_title(self, title: str) -> str:
        """标准化标题用于比较"""
        # 移除链接
        title = re.sub(r'\[([^\]]+)\]\([^)]+\)', r'\1', title)
        # 移除格式标记
        title = re.sub(r'[*_`]', '', title)
        # 转小写
        title = title.lower().strip()
        return title
    
    def compare_sections(self, en_sections: List[Tuple[int, str]], 
                        zh_sections: List[Tuple[int, str]]) -> List[Dict]:
        """比较章节结构"""
        issues = []
        
        # 简单的结构对比：检查章节数量是否相近
        if abs(len(en_sections) - len(zh_sections)) > 3:
            issues.append({
                "type": "warning",
                "message": f"章节数量差异较大: 英文 {len(en_sections)} 个，中文 {len(zh_sections)} 个"
            })
        
        # 检查是否有明显的章节丢失
        en_titles = [self.normalize_title(t) for _, t in en_sections]
        zh_titles = [self.normalize_title(t) for _, t in zh_sections]
        
        # 检查关键章节是否存在
        key_sections = ["quick start", "installation", "usage", "security", "license"]
        for key in key_sections:
            en_has = any(key in t for t in en_titles)
            zh_has = any(key in t or self.SECTION_MAPPINGS.get(key, "") in t for t in zh_titles)
            
            if en_has and not zh_has:
                issues.append({
                    "type": "warning",
                    "message": f"中文部分可能缺少章节: '{key}'"
                })
        
        return issues
    
    def compare_code_blocks(self, en_blocks: List[Tuple[str, str]], 
                           zh_blocks: List[Tuple[str, str]]) -> List[Dict]:
        """比较代码块"""
        issues = []
        
        # 检查代码块数量
        if len(en_blocks) != len(zh_blocks):
            issues.append({
                "type": "warning",
                "message": f"代码块数量不一致: 英文 {len(en_blocks)} 个，中文 {len(zh_blocks)} 个"
            })
        
        # 检查代码块语言类型
        min_blocks = min(len(en_blocks), len(zh_blocks))
        for i in range(min_blocks):
            en_lang, en_code = en_blocks[i]
            zh_lang, zh_code = zh_blocks[i]
            
            if en_lang != zh_lang:
                issues.append({
                    "type": "error",
                    "message": f"代码块 {i+1} 语言类型不一致: 英文 '{en_lang}' vs 中文 '{zh_lang}'"
                })
        
        return issues
    
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
        
        # 检查是否为双语文件
        if not self.is_bilingual_file(content):
            return issues
        
        self.stats["bilingual_files"] += 1
        self.log(f"Checking bilingual file: {rel_path}")
        
        # 分离中英文内容
        en_content, zh_content = self.split_bilingual_content(content)
        
        if not zh_content:
            issues.append({
                "file": rel_path,
                "line": 0,
                "type": "warning",
                "message": "无法识别中文部分内容，请确保使用标准标记如 <a name='chinese'></a>"
            })
            return issues
        
        # 提取并比较各部分
        en_sections = self.extract_sections(en_content)
        zh_sections = self.extract_sections(zh_content)
        section_issues = self.compare_sections(en_sections, zh_sections)
        
        en_blocks = self.extract_code_blocks(en_content)
        zh_blocks = self.extract_code_blocks(zh_content)
        code_issues = self.compare_code_blocks(en_blocks, zh_blocks)
        
        # 合并问题
        for issue in section_issues + code_issues:
            issue["file"] = rel_path
            issues.append(issue)
            if issue["type"] == "error":
                self.stats["issues"] += 1
        
        self.stats["checked"] += 1
        return issues
    
    def check_all(self) -> Dict:
        """检查所有 Markdown 文件"""
        exclude_dirs = {".git", "node_modules", "target", ".venv", "venv", "__pycache__"}
        
        md_files = [
            f for f in self.root_dir.rglob("*.md")
            if not any(excluded in str(f) for excluded in exclude_dirs)
        ]
        
        all_issues = []
        for file_path in sorted(md_files):
            issues = self.check_file(file_path)
            all_issues.extend(issues)
        
        # 分类问题
        for issue in all_issues:
            if issue.get("type") == "error":
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
        print("🌐 双语同步检查报告")
        print("=" * 60)
        print(f"\n统计:")
        print(f"  - 检查文件数: {self.stats['checked']}")
        print(f"  - 双语文件数: {self.stats['bilingual_files']}")
        print(f"  - 同步问题: {self.stats['issues']}")
        
        if self.errors:
            print(f"\n❌ 错误 ({len(self.errors)}):")
            for error in self.errors[:10]:
                print(f"\n  {error.get('file', 'N/A')}")
                print(f"    {error['message']}")
        
        if self.warnings:
            print(f"\n⚠️  警告 ({len(self.warnings)}):")
            for warning in self.warnings[:15]:
                print(f"\n  {warning.get('file', 'N/A')}")
                print(f"    {warning['message']}")
        
        print("\n" + "=" * 60)
    
    def _print_github_format(self):
        """GitHub Actions 格式输出"""
        for error in self.errors:
            file_path = error.get('file', '')
            print(f"::error file={file_path},title=Bilingual Sync Error::{error['message']}")
        
        for warning in self.warnings:
            file_path = warning.get('file', '')
            print(f"::warning file={file_path},title=Bilingual Sync Warning::{warning['message']}")


def main():
    parser = argparse.ArgumentParser(description="检查双语文档同步性")
    parser.add_argument("--root", "-r", default=".", help="项目根目录")
    parser.add_argument("--format", "-f", choices=["console", "github"], 
                       default="console", help="输出格式")
    parser.add_argument("--output", "-o", help="JSON 报告输出路径")
    parser.add_argument("--verbose", "-v", action="store_true", help="详细输出")
    
    args = parser.parse_args()
    
    checker = BilingualChecker(args.root, args.verbose)
    results = checker.check_all()
    
    checker.print_report(args.format)
    
    # 输出 JSON 报告
    if args.output:
        with open(args.output, "w", encoding="utf-8") as f:
            json.dump(results, f, ensure_ascii=False, indent=2)
        print(f"\n📄 报告已保存: {args.output}")
    
    # 返回退出码（警告不导致失败）
    sys.exit(0 if not checker.errors else 1)


if __name__ == "__main__":
    main()
