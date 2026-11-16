' 30 万次过程调用（ByRef 累加 + ByVal 参数 + 一个局部变量），用来看调用开销
' 跑法：qide-run run tools/bench/procedure_call.vb
' 注意累加值要留在 Long 范围内，否则会像 VB 一样报错误号 6（溢出）。
Dim i As Long, total As Long
For i = 1 To 300000
    AddTo total, i
Next
Sub AddTo(ByRef acc As Long, ByVal n As Long)
    Dim twice As Long
    twice = n * 2
    acc = acc + 1
End Sub
