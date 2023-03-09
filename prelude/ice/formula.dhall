let Prelude = ../package.dhall

let ice
    : Prelude.Formula.Type
    = { id = "ice"
      , name = Some "雾凇拼音"
      , use_hmm = True
      , dictionaries = [ "8105.dict.tsv" ]
      }

in  ice
