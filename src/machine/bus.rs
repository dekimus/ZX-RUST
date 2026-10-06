//! Contrato entre el CPU y el resto de la máquina.
//!
//! Decisión: el bus es dueño del reloj maestro. Cada método de acceso avanza el
//! reloj el coste del ciclo de máquina (más la contención futura, que depende
//! del T-state exacto del acceso). El CPU nunca suma retrasos por su cuenta;
//! solo usa `tick` para ciclos internos sin acceso al bus.

pub trait Bus {
    /// Ciclo M1 (fetch de opcode): 4 T-states base.
    fn fetch(&mut self, addr: u16) -> u8;
    /// Lectura de memoria: 3 T-states base.
    fn mem_read(&mut self, addr: u16) -> u8;
    /// Escritura de memoria: 3 T-states base.
    fn mem_write(&mut self, addr: u16, value: u8);
    /// Lectura de puerto: 4 T-states base.
    fn io_read(&mut self, port: u16) -> u8;
    /// Escritura de puerto: 4 T-states base.
    fn io_write(&mut self, port: u16, value: u8);
    /// Línea /INT activa en el instante actual.
    fn interrupt_line(&self) -> bool;
    /// Avanza ciclos internos del CPU sin dirección relevante en el bus (nunca contendidos).
    fn tick(&mut self, tstates: u32);
    /// Ciclos internos con `addr` en el bus de direcciones (p. ej. HL, IR, PC+n). En el 48K
    /// la ULA puede contender cada uno de ellos si `addr` cae en 0x4000..=0x7FFF.
    /// Por defecto, igual que `tick` (buses sin contención).
    fn tick_at(&mut self, _addr: u16, tstates: u32) {
        self.tick(tstates);
    }
    /// Contador maestro de T-states.
    fn tstate(&self) -> u64;
}
