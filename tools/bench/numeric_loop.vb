' 200 万次 x = x + 1 的纯数值循环
' 跑法：qide-run run tools/bench/numeric_loop.vb
Dim i As Long, x As Long
For i = 1 To 2000000
    x = x + 1
Next
