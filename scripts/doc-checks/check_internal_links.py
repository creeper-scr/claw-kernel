#!/usr/bin/env python3
"""
内部链接检查脚本
检查 Markdown 文件中的内部链接是否有效

检查项：
1. 相对路径链接是否存在
2. 锚点链接是否有效
3. 图片引用是否存在
4. 交叉引用一致性
"""

import argparse
import json
import os
import re
import sys
from pathlib import Path
from typing import Dict, List, Optional, Set, Tuple


class LinkChecker:
    """链接检查器"""
    
    def __init__(self, root_dir: str, verbose: bool = False):
        self.root_dir = Path(root_dir).resolve()
        self.verbose = verbose
        self.errors: List[Dict] = []
        self.warnings: List[Dict] = []
        self.stats = {"checked": 0, "valid": 0, "broken": 0, "external": 0}
        
        # 收集所有 Markdown 文件的锚点
        self.anchors: Dict[str, Set[str]] = {}
        self.md_files: Set[Path] = set()
        self._collect_files()
    
    def log(self, message: str):
        """日志输出"""
        if self.verbose:
            print(message)
    
    def _collect_files(self):
        """收集所有 Markdown 文件及其锚点"""
        exclude_dirs = {".git", "node_modules", "target", ".venv", "venv", "__pycache__"}
        
        for path in self.root_dir.rglob("*.md"):
            if any(excluded in str(path) for excluded in exclude_dirs):
                continue
            
            self.md_files.add(path)
            rel_path = path.relative_to(self.root_dir)
            self.anchors[str(rel_path)] = self._extract_anchors(path)
    
    def _extract_anchors(self, file_path: Path) -> Set[str]:
        """提取文件中的所有锚点"""
        anchors = set()
        
        try:
            content = file_path.read_text(encoding="utf-8")
        except Exception:
            return anchors
        
        # HTML 锚点 <a name="xxx">
        html_anchors = re.findall(r'<a\s+name=["\']([^"\']+)["\']', content)
        anchors.update(html_anchors)
        
        # Markdown 标题锚点
        # 提取所有标题
        headers = re.findall(r'^#{1,6}\s+(.+)$', content, re.MULTILINE)
        
        for header in headers:
            # GitHub 风格的锚点生成
            anchor = self._generate_anchor(header)
            anchors.add(anchor)
            
            # 也保留原始文本（用于兼容）
            anchors.add(header.lower().strip())
        
        return anchors
    
    def _generate_anchor(self, header: str) -> str:
        """生成 GitHub 风格的锚点"""
        # 转换为小写
        anchor = header.lower().strip()
        # 移除特殊字符，保留字母、数字、空格、连字符
        anchor = re.sub(r'[^\w\s-]', '', anchor)
        # 空格替换为连字符
        anchor = re.sub(r'\s+', '-', anchor)
        # 多个连字符合并
        anchor = re.sub(r'-+', '-', anchor)
        # 移除首尾连字符
        anchor = anchor.strip('-')
        return anchor
    
    def _extract_links(self, content: str) -> List[Tuple[str, str, int]]:
        """提取内容中的所有链接，返回 (链接文本, 链接目标, 行号)"""
        links = []
        
        # Markdown 链接 [text](url)
        for match in re.finditer(r'\[([^\]]+)\]\(([^)]+)\)', content):
            text = match.group(1)
            url = match.group(2)
            line_num = content[:match.start()].count('\n') + 1
            links.append((text, url, line_num))
        
        # HTML 链接 <a href="url">
        for match in re.finditer(r'<a\s+href=["\']([^"\']+)["\'][^>]*>([^<]*)</a>', content):
            url = match.group(1)
            text = match.group(2)
            line_num = content[:match.start()].count('\n') + 1
            links.append((text, url, line_num))
        
        # 图片引用 ![alt](url)
        for match in re.finditer(r'!\[([^\]]*)\]\(([^)]+)\)', content):
            alt = match.group(1)
            url = match.group(2)
            line_num = content[:match.start()].count('\n') + 1
            links.append((f"Image: {alt}", url, line_num))
        
        return links
    
    def _is_external_link(self, url: str) -> bool:
        """判断是否为外部链接"""
        return url.startswith(("http://", "https://", "mailto:", "ftp://"))
    
    def _resolve_link(self, url: str, current_file: Path) -> Tuple[Optional[Path], Optional[str]]:
        """
        解析链接目标
        返回 (目标文件路径, 锚点)
        """
        # 分离锚点
        if "#" in url:
            url_part, anchor = url.split("#", 1)
        else:
            url_part, anchor = url, None
        
        # 如果是纯锚点链接
        if not url_part:
            return current_file, anchor
        
        # 解析路径
        current_dir = current_file.parent
        target_path = current_dir / url_part
        target_path = target_path.resolve()
        
        # 检查文件是否存在
        if target_path.exists():
            return target_path, anchor
        
        # 尝试添加 .md 后缀
        if not target_path.suffix:
            md_path = Path(str(target_path) + ".md")
            if md_path.exists():
                return md_path, anchor
        
        # 检查是否是目录（指向 README/index）
        if target_path.is_dir():
            for index_file in ["README.md", "index.md", "readme.md"]:
                index_path = target_path / index_file
                if index_path.exists():
                    return index_path, anchor
        
        return None, anchor
    
    def check_file(self, file_path: Path) -> List[Dict]:
        """检查单个文件中的所有链接"""
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
        
        links = self._extract_links(content)
        rel_path = str(file_path.relative_to(self.root_dir))
        
        for text, url, line_num in links:
            self.stats["checked"] += 1
            
            # 跳过外部链接（在 CI 的其他 job 中检查）
            if self._is_external_link(url):
                self.stats["external"] += 1
                continue
            
            # 跳过纯锚点（同文件内跳转）
            if url.startswith("#"):
                anchor = url[1:]
                if anchor not in self.anchors.get(rel_path, set()):
                    # 尝试生成 GitHub 风格锚点
                    generated = self._generate_anchor(anchor.replace("-", " "))
                    if generated not in self.anchors.get(rel_path, set()):
                        issues.append({
                            "file": rel_path,
                            "line": line_num,
                            "type": "warning",
                            "message": f"未找到锚点: #{anchor}",
                            "context": f"[{text}]({url})"
                        })
                self.stats["valid"] += 1
                continue
            
            # 解析并检查链接
            target_file, anchor = self._resolve_link(url, file_path)
            
            if target_file is None:
                self.stats["broken"] += 1
                issues.append({
                    "file": rel_path,
                    "line": line_num,
                    "type": "error",
                    "message": f"链接目标不存在: {url}",
                    "context": f"[{text}]({url})"
                })
            else:
                # 检查锚点
                if anchor:
                    target_rel = str(target_file.relative_to(self.root_dir))
                    available_anchors = self.anchors.get(target_rel, set())
                    
                    if anchor not in available_anchors:
                        # 尝试 GitHub 风格锚点
                        generated = self._generate_anchor(anchor.replace("-", " "))
                        if generated not in available_anchors:
                            self.stats["broken"] += 1
                            issues.append({
                                "file": rel_path,
                                "line": line_num,
                                "type": "error",
                                "message": f"锚点不存在: {url}",
                                "context": f"[{text}]({url})"
                            })
                            continue
                
                self.stats["valid"] += 1
                self.log(f"  ✓ {url}")
        
        return issues
    
    def check_all(self) -> Dict:
        """检查所有文件的链接"""
        all_issues = []
        
        for file_path in sorted(self.md_files):
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
        print("🔗 内部链接检查报告")
        print("=" * 60)
        print(f"\n统计:")
        print(f"  - 检查链接数: {self.stats['checked']}")
        print(f"  - 有效: {self.stats['valid']}")
        print(f"  - 损坏: {self.stats['broken']}")
        print(f"  - 外部链接: {self.stats['external']}")
        
        if self.errors:
            print(f"\n❌ 错误 ({len(self.errors)}):")
            for error in self.errors[:20]:  # 只显示前20个
                print(f"  {error['file']}:{error['line']}: {error['message']}")
            if len(self.errors) > 20:
                print(f"  ... 还有 {len(self.errors) - 20} 个错误")
        
        if self.warnings:
            print(f"\n⚠️  警告 ({len(self.warnings)}):")
            for warning in self.warnings[:10]:  # 只显示前10个
                print(f"  {warning['file']}:{warning['line']}: {warning['message']}")
            if len(self.warnings) > 10:
                print(f"  ... 还有 {len(self.warnings) - 10} 个警告")
        
        print("\n" + "=" * 60)
    
    def _print_github_format(self):
        """GitHub Actions 格式输出"""
        for error in self.errors:
            print(f"::error file={error['file']},line={error['line']},title=Broken Link::{error['message']}")
        
        for warning in self.warnings:
            print(f"::warning file={warning['file']},line={warning['line']},title=Link Warning::{warning['message']}")


def main():
    parser = argparse.ArgumentParser(description="检查 Markdown 文件内部链接")
    parser.add_argument("--root", "-r", default=".", help="项目根目录")
    parser.add_argument("--format", "-f", choices=["console", "github"], 
                       default="console", help="输出格式")
    parser.add_argument("--output", "-o", help="JSON 报告输出路径")
    parser.add_argument("--verbose", "-v", action="store_true", help="详细输出")
    
    args = parser.parse_args()
    
    checker = LinkChecker(args.root, args.verbose)
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
