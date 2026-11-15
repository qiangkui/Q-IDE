' 1500 元素冒泡排序
' 跑法：qide-run run tools/bench/bubble_sort.vb
Dim i As Long, j As Long, tmp As Long
Dim a(1 To 1500) As Long
For i = 1 To 1500
    a(i) = ((1500 - i + 1) * 7919) Mod 10007
Next
For i = 1 To 1499
    For j = 1 To 1500 - i
        If a(j) > a(j + 1) Then
            tmp = a(j)
            a(j) = a(j + 1)
            a(j + 1) = tmp
        End If
    Next
Next