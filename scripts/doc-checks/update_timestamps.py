#!/usr/bin/env python3
"""
文档时间戳更新脚本
自动更新文档的 last_updated 字段
"""

import argparse
import re
import sys
from datetime import datetime
from pathlib import Path
from typing import List, Optional


class TimestampUpdater:
    """时间戳更新器"""
    
    def __init__(self, root_dir: str, verbose: bool = False):
        self.root_dir = Path(root_dir)
        self.verbose = verbose
        self.today = datetime.now().strftime("%Y-%m-%d")
        self.updated_files: List[str] = []
        self.skipped_files: List[str] = []
    
    def log(self, message: str):
        """日志输出"""
        if self.verbose:
            print(message)
    
    def update_file_timestamp(self, file_path: Path) -> bool:
        """更新单个文件的时间戳"""
        try:
            content = file_path.read_text(encoding="utf-8")
        except Exception as e:
            print(f"错误: 无法读取 {file_path}: {e}")
            return False
        
        # 检查是否有 YAML Front Matter
        if not content.startswith("---"):
            return False
        
        # 查找 last_updated 字段
        pattern = r'(last_updated:\s*)"\d{4}-\d{2}-\d{2}"'
        match = re.search(pattern, content)
        
        if not match:
            return False
        
        current_date = match.group(0).split('"')[1]
        
        # 如果已经是最新日期，跳过
        if current_date == self.today:
            self.skipped_files.append(str(file_path.relative_to(self.root_dir)))
            return False
        
        # 更新时间戳
        new_content = re.sub(pattern, f'\\1"{self.today}"', content)
        
        try:
            file_path.write_text(new_content, encoding="utf-8")
            self.updated_files.append(str(file_path.relative_to(self.root_dir)))
            self.log(f"已更新: {file_path.relative_to(self.root_dir)}")
            return True
        except Exception as e:
            print(f"错误: 无法写入 {file_path}: {e}")
            return False
    
    def find_stale_files(self, max_age_days: int) -> List[Path]:
        """查找过期的文档"""
        stale_files = []
        cutoff_date = datetime.now() - __import__('datetime').timedelta(days=max_age_days)
        
        exclude_dirs = {".git", "node_modules", "target", ".venv", "venv", "__pycache__"}
        
        for md_file in self.root_dir.rglob("*.md"):
            if any(excluded in str(md_file) for excluded in exclude_dirs):
                continue
            
            try:
                content = md_file.read_text(encoding="utf-8")
                
                # 提取 last_updated
                match = re.search(r'last_updated:\s*"(\d{4}-\d{2}-\d{2})"', content)
                if match:
                    doc_date = datetime.strptime(match.group(1), "%Y-%m-%d")
                    if doc_date < cutoff_date:
                        stale_files.append(md_file)
            except Exception:
                pass
        
        return stale_files
    
    def update_all(self, files_list: Optional[List[str]] = None):
        """更新所有文件或指定文件"""
        if files_list:
            # 从文件列表读取
            files_to_update = []
            for list_file in files_list:
                list_path = Path(list_file)
                if list_path.exists():
                    files_to_update.extend(
                        line.strip() for line in list_path.read_text().split('\n')
                        if line.strip()
                    )
            
            for file_str in files_to_update:
                file_path = self.root_dir / file_str
                if file_path.exists():
                    self.update_file_timestamp(file_path)
        else:
            # 更新所有文档
            exclude_dirs = {".git", "node_modules", "target", ".venv", "venv", "__pycache__"}
            
            for md_file in self.root_dir.rglob("*.md"):
                if any(excluded in str(md_file) for excluded in exclude_dirs):
                    continue
                self.update_file_timestamp(md_file)
    
    def print_summary(self):
        """打印更新摘要"""
        print("=" * 60)
        print("📅 文档时间戳更新摘要")
        print("=" * 60)
        print(f"\n更新日期: {self.today}")
        print(f"已更新: {len(self.updated_files)} 个文件")
        print(f"已是最新: {len(self.skipped_files)} 个文件")
        
        if self.updated_files:
            print("\n已更新的文件:")
            for f in self.updated_files:
                print(f"  - {f}")
        
        print("\n" + "=" * 60)


def main():
    parser = argparse.ArgumentParser(description="更新文档时间戳")
    parser.add_argument("--root", "-r", default=".", help="项目根目录")
    parser.add_argument("--files", "-f", nargs="+", help="要更新的文件列表")
    parser.add_argument("--check-only", action="store_true", 
                       help="仅检查过期文件，不更新")
    parser.add_argument("--max-age-days", "-d", type=int, default=7,
                       help="标记为过期的时间（天）")
    parser.add_argument("--output", "-o", help="过期文件列表输出路径")
    parser.add_argument("--verbose", "-v", action="store_true", help="详细输出")
    
    args = parser.parse_args()
    
    updater = TimestampUpdater(args.root, args.verbose)
    
    if args.check_only:
        stale_files = updater.find_stale_files(args.max_age_days)
        
        if stale_files:
            print(f"发现 {len(stale_files)} 个过期文档（超过 {args.max_age_days} 天未更新）:")
            for f in stale_files:
                print(f"  - {f.relative_to(args.root)}")
            
            if args.output:
                with open(args.output, "w") as f:
                    for path in stale_files:
                        f.write(str(path.relative_to(args.root)) + "\n")
                print(f"\n已保存到: {args.output}")
            
            sys.exit(0)
        else:
            print(f"所有文档都在 {args.max_age_days} 天内更新过")
            sys.exit(0)
    else:
        updater.update_all(args.files)
        updater.print_summary()


if __name__ == "__main__":
    main()
