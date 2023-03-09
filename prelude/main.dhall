let Prelude = ./package.dhall

let sunman = ./sunman/formula.dhall

let ice = ./ice/formula.dhall

let config
    : Prelude.Config
    = { formulas = [ sunman, ice ] }

in  config
