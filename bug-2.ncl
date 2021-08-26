let wrap = fun func src arg => func (src arg) in
let app = fun a => a ++ "app" in
let prep = fun b => "prep" ++ b in
let cat = fun x y => x ++ y in
(wrap (wrap cat prep) app) "bon" "jour"
