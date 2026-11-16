' 200 万次数组元素读取（与 `array_increment.vb` 对照，用来看读/写哪边贵）
' 跑法：qide-run run tools/bench/array_read.vb --max-steps 60000000
Dim i As Long, total As Long
Dim a(1 To 100) As Long
For i = 1 To 2000000
    total = total + a((i Mod 100) + 1)
Next
