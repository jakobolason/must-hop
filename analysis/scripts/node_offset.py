import sympy as sp

sp.init_printing(use_unicode=True)
# Define variables
nu, da, sg, sl, dur, base = sp.symbols('nu da sg sl dur base', real=True, positive=True)

curr = base + ((nu - da) * sg) * 1/sl

calc = ( dur - (curr % dur)) * sl/sg

simp = sp.simplify(calc)
sp.pprint(simp)
