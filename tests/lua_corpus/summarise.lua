-- The verbs that make a stream smaller: `group-by`, `count`, `distinct`, `stats`.
--
-- Everything oslo had before was selection. These are what make `ps | group-by user | count` a
-- query `ps | grep` cannot express.

oslo.register_tool {
  name = "sample",
  produces = "rows",
  rows = function()
    return {
      { user = "root", kind = "file", size = 100 },
      { user = "bres", kind = "file", size = 200 },
      { user = "root", kind = "dir", size = 300 },
      { user = "root", kind = "file", size = 400 },
    }
  end,
}

print("== how many ==")
oslo.proc.exec("sample | count | cols count")

print("== how many each ==")
oslo.proc.exec("sample | group-by user | count | cols user count")

print("== distinct by a column, keeping the first ==")
oslo.proc.exec("sample | distinct kind | cols kind size")

print("== the shape of a numeric column ==")
oslo.proc.exec("sample | stats size | cols count min max sum mean")

print("== composed with selection, both ways round ==")
oslo.proc.exec("sample | where 'user == \"root\"' | count | cols count")
oslo.proc.exec("sample | group-by kind | where 'count > 1' | cols kind count")

print("== an empty stream summarises without inventing anything ==")
oslo.proc.exec("sample | where 'user == \"nobody\"' | count | cols count")
oslo.proc.exec("sample | where 'user == \"nobody\"' | stats size | cols count")

--[[ expect
== how many ==
4
== how many each ==
root	3
bres	1
== distinct by a column, keeping the first ==
file	100
dir	300
== the shape of a numeric column ==
4	100	400	1000	250
== composed with selection, both ways round ==
3
file	3
== an empty stream summarises without inventing anything ==
0
0
]]
