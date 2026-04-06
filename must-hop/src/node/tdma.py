# %% Cell 1
import sympy as sp

# 1. Define the discrete time step index
k = sp.Symbol("k", integer=True)

# 2. Define the discrete-time signals as SymPy Functions
T_n = sp.Function("T_n")
T_G = sp.Function("T_G")
t_hw = sp.Function("t_hw")
S = sp.Function("S")
Delta_up = sp.Function("Delta_up")

# 3. Define the State vector (x) and Input vector (u)
# Using sp.Matrix to create column vectors
x_k = sp.Matrix([T_n(k), t_hw(k), S(k)])

u_k = sp.Matrix(
    [
        T_G(k),
        t_hw(
            k
        ),  # Note: t_hw[k] appears in both state and input vectors based on your prompt
        Delta_up(k),
    ]
)

# 4. Build the nested delay equation for T_n[k]
# T_n[k] = T_G[k] + (Delta_up[k] + (T_G[k] - (T_G[k-1] + (t_hw[k] - t_hw[k-1]*S[k])))) / 2
T_n_expr = (
    T_G(k)
    + (Delta_up(k) + (T_G(k) - (T_G(k - 1) + (t_hw(k) - t_hw(k - 1) * S(k))))) / 2
)

# Create the formal equation: T_n[k] = Expression
equation = sp.Eq(T_n(k), T_n_expr)

# 5. (Optional) Simplify or display the result
sp.pprint(equation)
