let bug | forall b. Dyn -> b -> Dyn = fun l fst => %length% l == 0 in
bug [] "a"
