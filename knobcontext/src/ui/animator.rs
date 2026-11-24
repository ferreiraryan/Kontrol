/// Constante de tempo exponencial (~175ms para 98% da transição de scroll).
const TAU_SCROLL_SECS: f32 = 0.045;
/// Constante de tempo para entrada do dock (~155ms para 98%).
const TAU_OPEN_SECS: f32 = 0.040;
/// Constante de tempo para transição entre telas (~140ms para 98%).
const TAU_SCREEN_SECS: f32 = 0.036;
/// Limiar de convergência abaixo do qual fazemos snap exato para repouso.
const EPSILON: f32 = 0.003;

/// Motor leve de animações exponenciais para o carrossel horizontal (60fps sob demanda).
///
/// Quando todas as propriedades atingem repouso (`is_active() == false`), o `Timer`
/// do Slint é desligado e o overlay consome 0% de CPU.
#[derive(Debug, Clone)]
pub struct CarouselAnimator {
    pub scroll_phase: f32,
    pub open_progress: f32,
    pub screen_progress: f32,
}

impl Default for CarouselAnimator {
    fn default() -> Self {
        Self::new()
    }
}

impl CarouselAnimator {
    pub fn new() -> Self {
        Self {
            scroll_phase: 0.0,
            open_progress: 1.0,
            screen_progress: 1.0,
        }
    }

    pub fn reset(&mut self) {
        self.scroll_phase = 0.0;
        self.open_progress = 1.0;
        self.screen_progress = 1.0;
    }

    pub fn trigger_open(&mut self, enabled: bool) {
        self.scroll_phase = 0.0;
        self.screen_progress = 1.0;
        self.open_progress = if enabled { 0.0 } else { 1.0 };
    }

    pub fn trigger_scroll(&mut self, delta: i32, enabled: bool) {
        if !enabled || delta == 0 {
            self.scroll_phase = 0.0;
            return;
        }
        // Acumula giros rápidos em rajada para inércia contínua sem travar.
        self.scroll_phase = (self.scroll_phase + (delta as f32)).clamp(-3.0, 3.0);
    }

    pub fn trigger_screen_transition(&mut self, enabled: bool) {
        self.scroll_phase = 0.0;
        self.screen_progress = if enabled { 0.0 } else { 1.0 };
    }

    pub fn is_active(&self) -> bool {
        self.scroll_phase.abs() >= EPSILON
            || (1.0 - self.open_progress).abs() >= EPSILON
            || (1.0 - self.screen_progress).abs() >= EPSILON
    }

    /// Avança as animações por `dt_secs` (clampado para evitar saltos em frames atrasados).
    /// Retorna `true` se ainda há animação em andamento, ou `false` quando convergiu.
    pub fn step(&mut self, dt_secs: f32) -> bool {
        let dt = dt_secs.clamp(0.001, 0.05);

        if self.scroll_phase.abs() >= EPSILON {
            self.scroll_phase *= (-dt / TAU_SCROLL_SECS).exp();
            if self.scroll_phase.abs() < EPSILON {
                self.scroll_phase = 0.0;
            }
        } else {
            self.scroll_phase = 0.0;
        }

        if (1.0 - self.open_progress).abs() >= EPSILON {
            self.open_progress = 1.0 - (1.0 - self.open_progress) * (-dt / TAU_OPEN_SECS).exp();
            if (1.0 - self.open_progress).abs() < EPSILON {
                self.open_progress = 1.0;
            }
        } else {
            self.open_progress = 1.0;
        }

        if (1.0 - self.screen_progress).abs() >= EPSILON {
            self.screen_progress =
                1.0 - (1.0 - self.screen_progress) * (-dt / TAU_SCREEN_SECS).exp();
            if (1.0 - self.screen_progress).abs() < EPSILON {
                self.screen_progress = 1.0;
            }
        } else {
            self.screen_progress = 1.0;
        }

        self.is_active()
    }
}

#[cfg(test)]
mod tests {
    use super::CarouselAnimator;

    #[test]
    fn scroll_accumulates_and_converges_smoothly_to_zero() {
        let mut anim = CarouselAnimator::new();
        assert!(!anim.is_active());

        anim.trigger_scroll(1, true);
        assert!(anim.is_active());
        assert!((anim.scroll_phase - 1.0).abs() < f32::EPSILON);

        // Giro rápido consecutivo acumula inércia
        anim.step(0.016);
        let mid = anim.scroll_phase;
        assert!(mid > 0.0 && mid < 1.0);
        anim.trigger_scroll(1, true);
        assert!(anim.scroll_phase > 1.0);

        // Converge completamente em menos de 400ms
        for _ in 0..30 {
            anim.step(0.016);
        }
        assert!(!anim.is_active());
        assert_eq!(anim.scroll_phase, 0.0);
    }

    #[test]
    fn disabled_animations_snap_immediately() {
        let mut anim = CarouselAnimator::new();
        anim.trigger_open(false);
        anim.trigger_scroll(1, false);
        anim.trigger_screen_transition(false);
        assert!(!anim.is_active());
        assert_eq!(anim.scroll_phase, 0.0);
        assert_eq!(anim.open_progress, 1.0);
        assert_eq!(anim.screen_progress, 1.0);
    }
}
