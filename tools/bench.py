"""跑 `tools/bench/*.vb` 里的解释器基准，支持「同一台机器、两个二进制交错跑」的 A/B 对比。

用法（在仓库根目录）：

    python tools/bench.py                          # 用 target/release/qide-run.exe 跑全部基准，打印最小值
    python tools/bench.py --runs 8                 # 每项跑 8 轮（默认 5）
    python tools/bench.py --only array             # 只跑文件名里含 `array` 的基准
    python tools/bench.py --old target/old.exe --new target/new.exe
                                                   # 两个二进制交错跑，打印各自的 min 与加速比

为什么必须交错：单跑一次的绝对值会随 CPU 频率/负载漂 ±20%，两个二进制分别跑完再比毫无意义。
这里的做法是「同一轮里先跑 A 再跑 B」，最后各取最小值（最小值比平均值稳定，受偶发卡顿影响小）。

计时取脚本自己报的「用时 X 秒」（`qide-run` 的结束行），不含进程启动与编译时间。
退出码：0 = 正常；2 = 找不到二进制或没有任何基准。
"""

import argparse
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
BENCH_DIR = os.path.join(HERE, "bench")
DEFAULT_BINARY = os.path.join(ROOT, "target", "release", "qide-run.exe")
# 数组/冒泡基准的语句数是千万级，步数上限要放大（默认上限不够）。
MAX_STEPS = "999999999"
ELAPSED = re.compile(r"用时\s*([0-9.]+)")


def scripts(only):
    if not os.path.isdir(BENCH_DIR):
        return []
    names = sorted(name for name in os.listdir(BENCH_DIR) if name.endswith(".vb"))
    if only:
        names = [name for name in names if only in name]
    return [os.path.join(BENCH_DIR, name) for name in names]


def run_once(binary, script, timeout=600):
    """跑一次脚本，返回它自己报的执行秒数；拿不到就返回 `None`。"""
    result = subprocess.run(
        [binary, "run", script, "--max-steps", MAX_STEPS],
        cwd=ROOT,
        capture_output=True,
        timeout=timeout,
    )
    text = (result.stdout + result.stderr).decode("utf-8", "replace")
    match = ELAPSED.search(text)
    if not match:
        # 中文被控制台按 GBK 解码时正则匹配不到，退回按 mbcs 再解一次。
        text = (result.stdout + result.stderr).decode("mbcs", "replace")
        match = ELAPSED.search(text)
    return float(match.group(1)) if match else None


def bench(binary, paths, runs):
    """交错跑：每一轮把每个脚本各跑一次，最后按脚本汇总最小值。"""
    best = {path: None for path in paths}
    for _ in range(runs):
        for path in paths:
            elapsed = run_once(binary, path)
            if elapsed is None:
                continue
            if best[path] is None or elapsed < best[path]:
                best[path] = elapsed
    return best


def describe(path):
    return os.path.splitext(os.path.basename(path))[0]


def main(argv):
    parser = argparse.ArgumentParser(description="解释器基准（支持交错 A/B）")
    parser.add_argument("--old", help="对比基准用的二进制（旧版）")
    parser.add_argument("--new", default=DEFAULT_BINARY, help="新版二进制（默认 target/release/qide-run.exe）")
    parser.add_argument("--runs", type=int, default=5, help="每个二进制的轮数（默认 5）")
    parser.add_argument("--only", help="只跑文件名里含该子串的基准")
    options = parser.parse_args(argv)

    paths = scripts(options.only)
    if not paths:
        print(f"{BENCH_DIR} 里没有基准脚本", file=sys.stderr)
        return 2
    for binary in filter(None, (options.old, options.new)):
        if not os.path.exists(binary):
            print(f"找不到 {binary}（先 `cargo build --release`）", file=sys.stderr)
            return 2

    if options.old:
        # 交错：同一轮里先旧后新，避免一段时间的负载/频率变化整体偏向某一边。
        old_best = {path: None for path in paths}
        new_best = {path: None for path in paths}
        for _ in range(options.runs):
            for path in paths:
                for binary, best in ((options.old, old_best), (options.new, new_best)):
                    elapsed = run_once(binary, path)
                    if elapsed is not None and (best[path] is None or elapsed < best[path]):
                        best[path] = elapsed
        print(f"{'基准':<20}{'旧':>10}{'新':>10}{'变化':>10}")
        for path in paths:
            before, after = old_best[path], new_best[path]
            if before is None or after is None:
                print(f"{describe(path):<20}{'—':>10}{'—':>10}{'取不到用时':>10}")
                continue
            change = (after - before) / before * 100.0
            print(f"{describe(path):<20}{before:>9.3f}s{after:>9.3f}s{change:>9.1f}%")
        return 0

    best = bench(options.new, paths, options.runs)
    print(f"{'基准':<20}{'最小值':>10}")
    for path in paths:
        elapsed = best[path]
        shown = "—" if elapsed is None else f"{elapsed:.3f}s"
        print(f"{describe(path):<20}{shown:>10}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
