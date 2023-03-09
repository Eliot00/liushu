let Prelude = ../package.dhall

let Formula = Prelude.Formula

let sunman =
      Formula::{
      , id = "sunman"
      , name = Some "山人全息"
      , dictionaries =
        [ "words.dict.tsv", "phrases.brief.dict.tsv", "phrases.core.dict.tsv" ]
      }

in  sunman
