' 200 万次数组元素自增
' 跑法：qide-run run tools/bench/array_increment.vb --max-steps 60000000
Dim i As Long
Dim a(1 To 100) As Long
For i = 1 To 2000000
    a((i Mod 100) + 1) = a((i Mod 100) + 1) + 1
Next
