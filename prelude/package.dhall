let Formula =
      { Type =
          { id : Text
          , name : Optional Text
          , use_hmm : Bool
          , dictionaries : List Text
          }
      , default = { name = None, use_hmm = False }
      }

let Config
    : Type
    = { formulas : List Formula.Type }

in  { Formula, Config }
