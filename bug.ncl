let bug | forall a b. List a -> b -> b = fun l fst => %length% l == 0 in
bug [] "a"
