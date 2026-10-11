-- "разблокируй компьютер": Windows cannot be unlocked by a normal program, so Jarvis says how to do it
-- (a program running in your session has no access to the Windows sign-in screen; and a voice command that
-- types a stored password would let anyone nearby open the computer)
say(T("Разблокировка", "Unlock"),
    T("Windows нельзя разблокировать программой. Введите пароль или PIN на экране входа, либо используйте отпечаток/лицо (Windows Hello).",
      "Windows cannot be unlocked by a program. Enter your password or PIN on the sign-in screen, or use fingerprint/face (Windows Hello)."),
    T("Разблокировать Windows я не могу. Введите пароль на экране входа.", "I cannot unlock Windows. Please enter your password on the sign-in screen."))
