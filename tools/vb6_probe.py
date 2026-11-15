"""用本机 VB6 真编译探测若干表达式的实际取值（不写黄金数据）。

用法（在仓库根目录）：
    python tools/vb6_probe.py "CStr(CDate(0))" "Format(1.005, \"0.00\")"
    python tools/vb6_probe.py --file 表达式清单.txt   # 每行一个，`#` 开头为注释

求值失败的表达式会标成 `<求值失败>`（VB6 里那一行没有输出）。
改动只影响 `target/vb6-probe` 下的临时工程。
"""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import vb6_golden


def main(argv):
    if not argv:
        sys.exit(__doc__)
    if argv[0] == "--file":
        with open(argv[1], encoding="utf-8") as handle:
            cases = [
                line.strip()
                for line in handle
                if line.strip() and not line.strip().startswith("#")
            ]
    else:
        cases = list(argv)

    vb6_golden.BUILD_DIR = os.path.join(vb6_golden.ROOT, "target", "vb6-probe")
    exe = vb6_golden.build(cases, ["Dim probe As Long"])
    values = vb6_golden.collect(exe)
    for index, expression in enumerate(cases):
        value = values.get(index)
        if value is None:
            print(f"{expression}  =>  <求值失败>")
        else:
            print(f"{expression}  =>  [{value}]")


if __name__ == "__main__":
    main(sys.argv[1:])
