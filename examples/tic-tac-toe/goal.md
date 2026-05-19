# Tic-Tac-Toe — Mission Goal

This mission: **2 players** play one game of Tic-Tac-Toe.

- **Referee**: hosts per `referee.md` — assigns X (first) and O (second) publicly, maintains the board, validates moves, declares the result.
- **Players**: play X or O per `player.md`; goal is to get three in a row (horizontal, vertical, or diagonal) before the opponent does.

Board uses numpad-style numbering:

```
 1 | 2 | 3
---+---+---
 4 | 5 | 6
---+---+---
 7 | 8 | 9
```

Win conditions:
- Either player gets three of their marks in a row → **that player wins**.
- All 9 cells filled with no three-in-a-row → **draw**.

The referee announces "Game on" to start. When the game ends, the referee shows the final board and announces the result.
